use crate::object::*;
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH, Duration};

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
        let month_days = [31, if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) { 29 } else { 28 },
                         31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
        month_days[..(m as usize - 1)].iter().sum::<i64>() + d - 1
    } else {
        d - 1
    };

    (y, m, d, hour, minute, second, wday, yday)
}

fn format_strftime(fmt: &str, y: i64, m: i64, d: i64, h: i64, min: i64, s: i64, wday: i64, yday: i64) -> String {
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
                Some('j') => result.push_str(&format!("{:03}", yday + 1)),
                Some('w') => result.push_str(&format!("{}", (wday + 1) % 7)),
                Some('u') => result.push_str(&format!("{}", if wday == 0 { 7 } else { wday })),
                Some('A') => {
                    let weekdays = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];
                    result.push_str(weekdays[wday as usize]);
                }
                Some('a') => {
                    let weekdays = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
                    result.push_str(weekdays[wday as usize]);
                }
                Some('B') => {
                    let months = ["January", "February", "March", "April", "May", "June",
                                  "July", "August", "September", "October", "November", "December"];
                    result.push_str(months[(m - 1) as usize]);
                }
                Some('b') | Some('h') => {
                    let months = ["Jan", "Feb", "Mar", "Apr", "May", "Jun",
                                  "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
                    result.push_str(months[(m - 1) as usize]);
                }
                Some('c') => result.push_str(&format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", y, m, d, h, min, s)),
                Some('x') => result.push_str(&format!("{:04}-{:02}-{:02}", y, m, d)),
                Some('X') => result.push_str(&format!("{:02}:{:02}:{:02}", h, min, s)),
                Some('%') => result.push('%'),
                Some(other) => { result.push('%'); result.push(other); }
                None => result.push('%'),
            }
        } else {
            result.push(c);
        }
    }
    result
}

pub fn create_time_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();

    macro_rules! time_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
                name: $name.to_string(),
                func: $func,
            }));
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
        let secs = if args.is_empty() { 0.0 } else { args[0].as_f64().unwrap_or(0.0) };
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

    // gmtime(secs=None) -> struct_time
    time_func!("gmtime", |args| {
        let secs = if !args.is_empty() { args[0].as_i64().unwrap_or(0) } else {
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64
        };
        let (y, m, d, h, min, s, wday, yday) = epoch_to_ymd(secs);
        Ok(py_tuple(vec![
            py_int(y), py_int(m), py_int(d),
            py_int(h), py_int(min), py_int(s),
            py_int(wday), py_int(yday), py_int(0),
        ]))
    });

    // localtime(secs=None) -> struct_time
    time_func!("localtime", |args| {
        let secs = if !args.is_empty() { args[0].as_i64().unwrap_or(0) } else {
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64
        };
        let (y, m, d, h, min, s, wday, yday) = epoch_to_ymd(secs);
        Ok(py_tuple(vec![
            py_int(y), py_int(m), py_int(d),
            py_int(h), py_int(min), py_int(s),
            py_int(wday), py_int(yday), py_int(0),
        ]))
    });

    // strftime(format, struct_time) -> string
    time_func!("strftime", |args| {
        let fmt = if args.len() > 0 { args[0].str() } else { "%c".to_string() };
        let (y, m, d, h, min, s, wday, yday) = if args.len() > 1 {
            let t = &args[1];
            if let PyObject::Tuple(items) = &*t.borrow() {
                let y = items.get(0).and_then(|v| v.as_i64()).unwrap_or(2025);
                let m = items.get(1).and_then(|v| v.as_i64()).unwrap_or(1);
                let d = items.get(2).and_then(|v| v.as_i64()).unwrap_or(1);
                let h = items.get(3).and_then(|v| v.as_i64()).unwrap_or(0);
                let min = items.get(4).and_then(|v| v.as_i64()).unwrap_or(0);
                let s = items.get(5).and_then(|v| v.as_i64()).unwrap_or(0);
                let wday = items.get(6).and_then(|v| v.as_i64()).unwrap_or(0);
                let yday = items.get(7).and_then(|v| v.as_i64()).unwrap_or(0);
                (y, m, d, h, min, s, wday, yday)
            } else {
                let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64;
                epoch_to_ymd(now)
            }
        } else {
            let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64;
            epoch_to_ymd(now)
        };
        Ok(py_str(&format_strftime(&fmt, y, m, d, h, min, s, wday, yday)))
    });

    // strptime(string, format) -> struct_time
    time_func!("strptime", |args| {
        let _string = if args.len() > 0 { args[0].str() } else { String::new() };
        let _fmt = if args.len() > 1 { args[1].str() } else { "%c".to_string() };
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64;
        let (y, m, d, h, min, s, wday, yday) = epoch_to_ymd(now);
        Ok(py_tuple(vec![
            py_int(y), py_int(m), py_int(d),
            py_int(h), py_int(min), py_int(s),
            py_int(wday), py_int(yday), py_int(0),
        ]))
    });

    // perf_counter() -> float (high-resolution monotonic)
    time_func!("perf_counter", |_args| {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::from_secs(0));
        Ok(py_float(now.as_secs_f64()))
    });

    // Constants
    d.insert("altzone".to_string(), py_int(0));
    d.insert("daylight".to_string(), py_int(0));
    d.insert("timezone".to_string(), py_int(0));
    d.insert("tzname".to_string(), py_tuple(vec![py_str("UTC"), py_str("UTC")]));

    // struct_time named tuple stub (use tuple)
    d.insert("struct_time".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "struct_time".to_string(),
        func: |args| {
            if args.is_empty() { Ok(py_tuple(vec![])) }
            else { Ok(args[0].clone()) }
        },
    }));

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
    if month == 2 && is_leap_year(year) { 29 } else { DAYS_IN_MONTH[month as usize] }
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
    days_before_month_table()[month as usize] + if month > 2 && is_leap_year(year) { 1 } else { 0 }
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
    let mut preceding = t[month as usize] + if month > 2 && is_leap_year(year) { 1 } else { 0 };
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
    if pos + 44 > data.len() { return None; }
    let isutcnt = read_i32_be(data, pos + 20) as usize;
    let isstdcnt = read_i32_be(data, pos + 24) as usize;
    let leapcnt = read_i32_be(data, pos + 28) as usize;
    let timecnt = read_i32_be(data, pos + 32) as usize;
    let typecnt = read_i32_be(data, pos + 36) as usize;
    let charcnt = read_i32_be(data, pos + 40) as usize;
    let mut p = pos + 44;

    let mut transitions = Vec::with_capacity(timecnt);
    for _ in 0..timecnt {
        if p + time_size > data.len() { return None; }
        let t = if time_size == 8 { read_i64_be(data, p) } else { read_i32_be(data, p) as i64 };
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
        if p + 6 > data.len() { return None; }
        let utoff = read_i32_be(data, p);
        let isdst = data[p + 4] != 0;
        let desigidx = data[p + 5] as usize;
        ttinfo_raw.push((utoff, isdst, desigidx));
        p += 6;
    }
    if p + charcnt > data.len() { return None; }
    let charpool = &data[p..p + charcnt];
    p += charcnt;
    p += leapcnt * (time_size + 4);
    p += isstdcnt;
    p += isutcnt;

    let ttinfos: Vec<(i32, bool, String)> = ttinfo_raw
        .into_iter()
        .map(|(utoff, isdst, idx)| {
            let desig = if idx < charpool.len() {
                let end = charpool[idx..].iter().position(|&b| b == 0).map(|o| idx + o).unwrap_or(charpool.len());
                String::from_utf8_lossy(&charpool[idx..end]).to_string()
            } else {
                String::new()
            };
            (utoff, isdst, desig)
        })
        .collect();

    Some((ParsedTz { transitions, trans_types, ttinfos }, p))
}

fn parse_tzif(bytes: &[u8]) -> Option<ParsedTz> {
    if bytes.len() < 44 || &bytes[0..4] != b"TZif" { return None; }
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
        None => tz.ttinfos.iter().find(|t| !t.1).cloned().unwrap_or_else(|| tz.ttinfos[0].clone()),
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
    if let PyObject::Instance { dict, .. } = &*obj.borrow() { dict.get(name).cloned() } else { None }
}

fn inst_get_i64(obj: &PyObjectRef, name: &str) -> i64 {
    inst_get(obj, name).and_then(|v| v.as_i64()).unwrap_or(0)
}

fn instance_type_name(obj: &PyObjectRef) -> String {
    if let PyObject::Instance { typ, .. } = &*obj.borrow() {
        if let PyObject::Type { name, .. } = &*typ.borrow() { return name.clone(); }
    }
    String::new()
}

/// UTC offset in seconds for `tzinfo`, evaluated at the naive wall-clock
/// instant given by `ordinal`/`day_seconds` (days since 0001-01-01 and
/// seconds since local midnight). Only understands this module's own
/// `timezone` and `zoneinfo.ZoneInfo` — see module-level doc comment.
fn get_utcoffset_seconds(tzinfo: &PyObjectRef, ordinal: i64, day_seconds: i64) -> Option<i64> {
    if matches!(tzinfo, PyObjectRef::None) { return None; }
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
    if matches!(tzinfo, PyObjectRef::None) { return None; }
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
    if offset_seconds == 0 { return "UTC".to_string(); }
    let sign = if offset_seconds < 0 { '-' } else { '+' };
    let abs = offset_seconds.abs();
    let h = abs / 3600;
    let m = (abs % 3600) / 60;
    if m == 0 { format!("UTC{}{:02}", sign, h) } else { format!("UTC{}{:02}:{:02}", sign, h, m) }
}

fn format_offset_iso(offset_seconds: i64) -> String {
    let sign = if offset_seconds < 0 { '-' } else { '+' };
    let abs = offset_seconds.abs();
    let h = abs / 3600;
    let m = (abs % 3600) / 60;
    let s = abs % 60;
    if s == 0 { format!("{}{:02}:{:02}", sign, h, m) } else { format!("{}{:02}:{:02}:{:02}", sign, h, m, s) }
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
        self.pos.get(idx).cloned().or_else(|| self.kw.get(name).cloned())
    }

    fn get_i64(&self, idx: usize, name: &str, default: i64) -> i64 {
        self.get(idx, name).and_then(|v| v.as_i64()).unwrap_or(default)
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
    if let Some(t) = existing { return t; }
    let mut type_dict: HashMap<String, PyObjectRef> = HashMap::new();
    macro_rules! bf {
        ($name:expr, $f:expr) => {
            PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $f })
        };
    }
    type_dict.insert("utcoffset".to_string(), bf!("utcoffset", |_args| Err(PyError::runtime_error("tzinfo subclasses must override utcoffset()"))));
    type_dict.insert("dst".to_string(), bf!("dst", |_args| Err(PyError::runtime_error("tzinfo subclasses must override dst()"))));
    type_dict.insert("tzname".to_string(), bf!("tzname", |_args| Err(PyError::runtime_error("tzinfo subclasses must override tzname()"))));
    let typ = PyObjectRef::new(PyObject::Type { name: "tzinfo".to_string(), dict: type_dict, bases: vec![], mro: vec![] });
    TZINFO_TYPE.with(|c| { *c.borrow_mut() = Some(typ.clone()); });
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
        out.push_str(&format!("{} day{}, ", d, if d.abs() == 1 { "" } else { "s" }));
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
            PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $f })
        };
    }

    type_dict.insert("__init__".to_string(), bf!("__init__", |args| {
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
            dict.insert("days".to_string(), py_int(d));
            dict.insert("seconds".to_string(), py_int(s));
            dict.insert("microseconds".to_string(), py_int(us));
        }
        Ok(py_none())
    }));
    type_dict.insert("total_seconds".to_string(), bf!("total_seconds", |args| {
        let d = inst_get_i64(&args[0], "days");
        let s = inst_get_i64(&args[0], "seconds");
        let us = inst_get_i64(&args[0], "microseconds");
        Ok(py_float(d as f64 * 86400.0 + s as f64 + us as f64 / 1_000_000.0))
    }));
    type_dict.insert("__str__".to_string(), bf!("__str__", |args| Ok(py_str(&timedelta_str(&args[0])))));
    type_dict.insert("__repr__".to_string(), bf!("__repr__", |args| {
        let d = inst_get_i64(&args[0], "days");
        let s = inst_get_i64(&args[0], "seconds");
        let us = inst_get_i64(&args[0], "microseconds");
        let mut parts = vec![];
        if d != 0 { parts.push(format!("days={}", d)); }
        if s != 0 { parts.push(format!("seconds={}", s)); }
        if us != 0 { parts.push(format!("microseconds={}", us)); }
        if parts.is_empty() { parts.push("0".to_string()); }
        Ok(py_str(&format!("datetime.timedelta({})", parts.join(", "))))
    }));
    type_dict.insert("__eq__".to_string(), bf!("__eq__", |args| {
        if instance_type_name(&args[1]) != "timedelta" { return Ok(py_bool(false)); }
        Ok(py_bool(timedelta_total_us(&args[0]) == timedelta_total_us(&args[1])))
    }));
    type_dict.insert("__lt__".to_string(), bf!("__lt__", |args| {
        if instance_type_name(&args[1]) != "timedelta" { return Err(PyError::type_error("'<' not supported between instances of 'timedelta' and other type")); }
        Ok(py_bool(timedelta_total_us(&args[0]) < timedelta_total_us(&args[1])))
    }));
    type_dict.insert("__le__".to_string(), bf!("__le__", |args| {
        if instance_type_name(&args[1]) != "timedelta" { return Err(PyError::type_error("'<=' not supported between instances of 'timedelta' and other type")); }
        Ok(py_bool(timedelta_total_us(&args[0]) <= timedelta_total_us(&args[1])))
    }));
    type_dict.insert("__gt__".to_string(), bf!("__gt__", |args| {
        if instance_type_name(&args[1]) != "timedelta" { return Err(PyError::type_error("'>' not supported between instances of 'timedelta' and other type")); }
        Ok(py_bool(timedelta_total_us(&args[0]) > timedelta_total_us(&args[1])))
    }));
    type_dict.insert("__ge__".to_string(), bf!("__ge__", |args| {
        if instance_type_name(&args[1]) != "timedelta" { return Err(PyError::type_error("'>=' not supported between instances of 'timedelta' and other type")); }
        Ok(py_bool(timedelta_total_us(&args[0]) >= timedelta_total_us(&args[1])))
    }));
    type_dict.insert("__hash__".to_string(), bf!("__hash__", |args| Ok(py_int(timedelta_total_us(&args[0]) as i64))));
    type_dict.insert("__bool__".to_string(), bf!("__bool__", |args| Ok(py_bool(timedelta_total_us(&args[0]) != 0))));
    type_dict.insert("__add__".to_string(), bf!("__add__", |args| {
        if instance_type_name(&args[1]) != "timedelta" { return Err(PyError::type_error("unsupported operand type(s) for +: 'timedelta' and other type")); }
        Ok(make_timedelta_from_us(timedelta_total_us(&args[0]) + timedelta_total_us(&args[1])))
    }));
    type_dict.insert("__radd__".to_string(), bf!("__radd__", |args| {
        if instance_type_name(&args[1]) != "timedelta" { return Err(PyError::type_error("unsupported operand type(s) for +: 'timedelta' and other type")); }
        Ok(make_timedelta_from_us(timedelta_total_us(&args[0]) + timedelta_total_us(&args[1])))
    }));
    type_dict.insert("__sub__".to_string(), bf!("__sub__", |args| {
        if instance_type_name(&args[1]) != "timedelta" { return Err(PyError::type_error("unsupported operand type(s) for -: 'timedelta' and other type")); }
        Ok(make_timedelta_from_us(timedelta_total_us(&args[0]) - timedelta_total_us(&args[1])))
    }));
    type_dict.insert("__rsub__".to_string(), bf!("__rsub__", |args| {
        if instance_type_name(&args[1]) != "timedelta" { return Err(PyError::type_error("unsupported operand type(s) for -: 'timedelta' and other type")); }
        Ok(make_timedelta_from_us(timedelta_total_us(&args[1]) - timedelta_total_us(&args[0])))
    }));
    type_dict.insert("__neg__".to_string(), bf!("__neg__", |args| Ok(make_timedelta_from_us(-timedelta_total_us(&args[0])))));
    type_dict.insert("__pos__".to_string(), bf!("__pos__", |args| Ok(make_timedelta_from_us(timedelta_total_us(&args[0])))));
    type_dict.insert("__abs__".to_string(), bf!("__abs__", |args| Ok(make_timedelta_from_us(timedelta_total_us(&args[0]).abs()))));
    type_dict.insert("__mul__".to_string(), bf!("__mul__", |args| {
        let factor = args[1].as_f64().ok_or_else(|| PyError::type_error("unsupported operand type(s) for *"))?;
        Ok(make_timedelta_from_us((timedelta_total_us(&args[0]) as f64 * factor).round() as i128))
    }));
    type_dict.insert("__rmul__".to_string(), bf!("__rmul__", |args| {
        let factor = args[1].as_f64().ok_or_else(|| PyError::type_error("unsupported operand type(s) for *"))?;
        Ok(make_timedelta_from_us((timedelta_total_us(&args[0]) as f64 * factor).round() as i128))
    }));
    type_dict.insert("__truediv__".to_string(), bf!("__truediv__", |args| {
        if instance_type_name(&args[1]) == "timedelta" {
            let a = timedelta_total_us(&args[0]) as f64;
            let b = timedelta_total_us(&args[1]) as f64;
            return Ok(py_float(a / b));
        }
        let divisor = args[1].as_f64().ok_or_else(|| PyError::type_error("unsupported operand type(s) for /"))?;
        Ok(make_timedelta_from_us((timedelta_total_us(&args[0]) as f64 / divisor).round() as i128))
    }));
    type_dict.insert("__floordiv__".to_string(), bf!("__floordiv__", |args| {
        if instance_type_name(&args[1]) == "timedelta" {
            let a = timedelta_total_us(&args[0]);
            let b = timedelta_total_us(&args[1]);
            return Ok(py_int((a / b) as i64));
        }
        let divisor = args[1].as_i64().ok_or_else(|| PyError::type_error("unsupported operand type(s) for //"))?;
        Ok(make_timedelta_from_us(timedelta_total_us(&args[0]) / divisor as i128))
    }));

    PyObjectRef::new(PyObject::Type { name: "timedelta".to_string(), dict: type_dict, bases: vec![], mro: vec![] })
}

fn make_timedelta_with_type(typ: PyObjectRef, days: i64, seconds: i64, microseconds: i64) -> PyObjectRef {
    let mut dict = HashMap::new();
    dict.insert("days".to_string(), py_int(days));
    dict.insert("seconds".to_string(), py_int(seconds));
    dict.insert("microseconds".to_string(), py_int(microseconds));
    PyObjectRef::new(PyObject::Instance { typ, dict })
}

fn make_timedelta(days: i64, seconds: i64, microseconds: i64) -> PyObjectRef {
    let (days, seconds, microseconds) = normalize_timedelta(days, seconds, microseconds);
    make_timedelta_with_type(get_timedelta_type(), days, seconds, microseconds)
}

fn get_timedelta_type() -> PyObjectRef {
    let existing = TIMEDELTA_TYPE.with(|c| c.borrow().clone());
    if let Some(t) = existing { return t; }
    let typ = build_timedelta_type();
    let min_inst = make_timedelta_with_type(typ.clone(), -999_999_999, 0, 0);
    let max_inst = make_timedelta_with_type(typ.clone(), 999_999_999, 86399, 999_999);
    let res_inst = make_timedelta_with_type(typ.clone(), 0, 0, 1);
    if let PyObject::Type { dict, .. } = &mut *typ.borrow_mut() {
        dict.insert("min".to_string(), min_inst);
        dict.insert("max".to_string(), max_inst);
        dict.insert("resolution".to_string(), res_inst);
    }
    TIMEDELTA_TYPE.with(|c| { *c.borrow_mut() = Some(typ.clone()); });
    typ
}

// ---- date ----

fn date_ordinal(obj: &PyObjectRef) -> i64 {
    ymd_to_ordinal(inst_get_i64(obj, "year"), inst_get_i64(obj, "month"), inst_get_i64(obj, "day"))
}

fn make_date_from_ordinal(ord: i64) -> PyObjectRef {
    let (y, m, d) = ordinal_to_ymd(ord);
    make_date(y, m, d)
}

fn build_date_type() -> PyObjectRef {
    let mut type_dict: HashMap<String, PyObjectRef> = HashMap::new();
    macro_rules! bf {
        ($name:expr, $f:expr) => {
            PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $f })
        };
    }

    type_dict.insert("__init__".to_string(), bf!("__init__", |args| {
        let ctor = CtorArgs::parse(&args[1..]);
        let year = ctor.get_i64(0, "year", 1);
        let month = ctor.get_i64(1, "month", 1);
        let day = ctor.get_i64(2, "day", 1);
        if !(1..=9999).contains(&year) { return Err(PyError::value_error("year out of range")); }
        if !(1..=12).contains(&month) { return Err(PyError::value_error("month must be in 1..12")); }
        if !(1..=days_in_month(year, month)).contains(&day) { return Err(PyError::value_error("day is out of range for month")); }
        if let PyObject::Instance { dict, .. } = &mut *args[0].borrow_mut() {
            dict.insert("year".to_string(), py_int(year));
            dict.insert("month".to_string(), py_int(month));
            dict.insert("day".to_string(), py_int(day));
        }
        Ok(py_none())
    }));
    type_dict.insert("isoformat".to_string(), bf!("isoformat", |args| {
        Ok(py_str(&format!("{:04}-{:02}-{:02}", inst_get_i64(&args[0], "year"), inst_get_i64(&args[0], "month"), inst_get_i64(&args[0], "day"))))
    }));
    type_dict.insert("__str__".to_string(), bf!("__str__", |args| {
        Ok(py_str(&format!("{:04}-{:02}-{:02}", inst_get_i64(&args[0], "year"), inst_get_i64(&args[0], "month"), inst_get_i64(&args[0], "day"))))
    }));
    type_dict.insert("__repr__".to_string(), bf!("__repr__", |args| {
        Ok(py_str(&format!("datetime.date({}, {}, {})", inst_get_i64(&args[0], "year"), inst_get_i64(&args[0], "month"), inst_get_i64(&args[0], "day"))))
    }));
    type_dict.insert("weekday".to_string(), bf!("weekday", |args| Ok(py_int(weekday_from_ordinal(date_ordinal(&args[0]))))));
    type_dict.insert("isoweekday".to_string(), bf!("isoweekday", |args| Ok(py_int(weekday_from_ordinal(date_ordinal(&args[0])) + 1))));
    type_dict.insert("toordinal".to_string(), bf!("toordinal", |args| Ok(py_int(date_ordinal(&args[0])))));
    type_dict.insert("timetuple".to_string(), bf!("timetuple", |args| {
        let ord = date_ordinal(&args[0]);
        let year = inst_get_i64(&args[0], "year");
        let wday = weekday_from_ordinal(ord);
        let yday = day_of_year(year, ord);
        Ok(py_tuple(vec![
            py_int(year), py_int(inst_get_i64(&args[0], "month")), py_int(inst_get_i64(&args[0], "day")),
            py_int(0), py_int(0), py_int(0), py_int(wday), py_int(yday + 1), py_int(-1),
        ]))
    }));
    type_dict.insert("strftime".to_string(), bf!("strftime", |args| {
        let fmt = if args.len() > 1 { args[1].str() } else { "%Y-%m-%d".to_string() };
        let ord = date_ordinal(&args[0]);
        let year = inst_get_i64(&args[0], "year");
        Ok(py_str(&format_strftime(&fmt, year, inst_get_i64(&args[0], "month"), inst_get_i64(&args[0], "day"), 0, 0, 0, weekday_from_ordinal(ord), day_of_year(year, ord))))
    }));
    type_dict.insert("replace".to_string(), bf!("replace", |args| {
        let ctor = CtorArgs::parse(&args[1..]);
        let year = ctor.get(0, "year").and_then(|v| v.as_i64()).unwrap_or_else(|| inst_get_i64(&args[0], "year"));
        let month = ctor.get(1, "month").and_then(|v| v.as_i64()).unwrap_or_else(|| inst_get_i64(&args[0], "month"));
        let day = ctor.get(2, "day").and_then(|v| v.as_i64()).unwrap_or_else(|| inst_get_i64(&args[0], "day"));
        Ok(make_date(year, month, day))
    }));
    type_dict.insert("__eq__".to_string(), bf!("__eq__", |args| {
        if instance_type_name(&args[1]) != "date" { return Ok(py_bool(false)); }
        Ok(py_bool(date_ordinal(&args[0]) == date_ordinal(&args[1])))
    }));
    type_dict.insert("__lt__".to_string(), bf!("__lt__", |args| {
        if instance_type_name(&args[1]) != "date" { return Err(PyError::type_error("'<' not supported between instances of 'date' and other type")); }
        Ok(py_bool(date_ordinal(&args[0]) < date_ordinal(&args[1])))
    }));
    type_dict.insert("__le__".to_string(), bf!("__le__", |args| {
        if instance_type_name(&args[1]) != "date" { return Err(PyError::type_error("'<=' not supported between instances of 'date' and other type")); }
        Ok(py_bool(date_ordinal(&args[0]) <= date_ordinal(&args[1])))
    }));
    type_dict.insert("__gt__".to_string(), bf!("__gt__", |args| {
        if instance_type_name(&args[1]) != "date" { return Err(PyError::type_error("'>' not supported between instances of 'date' and other type")); }
        Ok(py_bool(date_ordinal(&args[0]) > date_ordinal(&args[1])))
    }));
    type_dict.insert("__ge__".to_string(), bf!("__ge__", |args| {
        if instance_type_name(&args[1]) != "date" { return Err(PyError::type_error("'>=' not supported between instances of 'date' and other type")); }
        Ok(py_bool(date_ordinal(&args[0]) >= date_ordinal(&args[1])))
    }));
    type_dict.insert("__hash__".to_string(), bf!("__hash__", |args| Ok(py_int(date_ordinal(&args[0])))));
    type_dict.insert("__add__".to_string(), bf!("__add__", |args| {
        if instance_type_name(&args[1]) != "timedelta" { return Err(PyError::type_error("unsupported operand type(s) for +: 'date' and other type")); }
        Ok(make_date_from_ordinal(date_ordinal(&args[0]) + inst_get_i64(&args[1], "days")))
    }));
    type_dict.insert("__radd__".to_string(), bf!("__radd__", |args| {
        if instance_type_name(&args[1]) != "timedelta" { return Err(PyError::type_error("unsupported operand type(s) for +: 'date' and other type")); }
        Ok(make_date_from_ordinal(date_ordinal(&args[0]) + inst_get_i64(&args[1], "days")))
    }));
    type_dict.insert("__sub__".to_string(), bf!("__sub__", |args| {
        match instance_type_name(&args[1]).as_str() {
            "timedelta" => Ok(make_date_from_ordinal(date_ordinal(&args[0]) - inst_get_i64(&args[1], "days"))),
            "date" => Ok(make_timedelta(date_ordinal(&args[0]) - date_ordinal(&args[1]), 0, 0)),
            _ => Err(PyError::type_error("unsupported operand type(s) for -: 'date' and other type")),
        }
    }));
    type_dict.insert("today".to_string(), bf!("today", |_args| {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        let (y, m, d, _, _, _, _, _) = epoch_to_ymd(now.as_secs() as i64);
        Ok(make_date(y, m, d))
    }));
    type_dict.insert("fromordinal".to_string(), bf!("fromordinal", |args| {
        let n = if !args.is_empty() { args[0].as_i64().unwrap_or(1) } else { 1 };
        Ok(make_date_from_ordinal(n))
    }));
    type_dict.insert("fromtimestamp".to_string(), bf!("fromtimestamp", |args| {
        let ts = if !args.is_empty() { args[0].as_f64().unwrap_or(0.0) } else { 0.0 };
        let (y, m, d, _, _, _, _, _) = epoch_to_ymd(ts as i64);
        Ok(make_date(y, m, d))
    }));
    type_dict.insert("fromisoformat".to_string(), bf!("fromisoformat", |args| {
        let s = if !args.is_empty() { args[0].str() } else { String::new() };
        let parts: Vec<&str> = s.splitn(3, '-').collect();
        if parts.len() != 3 { return Err(PyError::value_error("Invalid isoformat string")); }
        let y: i64 = parts[0].parse().map_err(|_| PyError::value_error("Invalid isoformat string"))?;
        let m: i64 = parts[1].parse().map_err(|_| PyError::value_error("Invalid isoformat string"))?;
        let d: i64 = parts[2].parse().map_err(|_| PyError::value_error("Invalid isoformat string"))?;
        Ok(make_date(y, m, d))
    }));

    PyObjectRef::new(PyObject::Type { name: "date".to_string(), dict: type_dict, bases: vec![], mro: vec![] })
}

fn make_date(year: i64, month: i64, day: i64) -> PyObjectRef {
    let typ = get_date_type();
    let mut dict = HashMap::new();
    dict.insert("year".to_string(), py_int(year));
    dict.insert("month".to_string(), py_int(month));
    dict.insert("day".to_string(), py_int(day));
    PyObjectRef::new(PyObject::Instance { typ, dict })
}

fn get_date_type() -> PyObjectRef {
    let existing = DATE_TYPE.with(|c| c.borrow().clone());
    if let Some(t) = existing { return t; }
    let typ = build_date_type();
    let min_inst = PyObjectRef::new(PyObject::Instance {
        typ: typ.clone(),
        dict: HashMap::from([("year".to_string(), py_int(1)), ("month".to_string(), py_int(1)), ("day".to_string(), py_int(1))]),
    });
    let max_inst = PyObjectRef::new(PyObject::Instance {
        typ: typ.clone(),
        dict: HashMap::from([("year".to_string(), py_int(9999)), ("month".to_string(), py_int(12)), ("day".to_string(), py_int(31))]),
    });
    let res_inst = make_timedelta(1, 0, 0);
    if let PyObject::Type { dict, .. } = &mut *typ.borrow_mut() {
        dict.insert("min".to_string(), min_inst);
        dict.insert("max".to_string(), max_inst);
        dict.insert("resolution".to_string(), res_inst);
    }
    DATE_TYPE.with(|c| { *c.borrow_mut() = Some(typ.clone()); });
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
            PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $f })
        };
    }

    type_dict.insert("__init__".to_string(), bf!("__init__", |args| {
        let ctor = CtorArgs::parse(&args[1..]);
        let hour = ctor.get_i64(0, "hour", 0);
        let minute = ctor.get_i64(1, "minute", 0);
        let second = ctor.get_i64(2, "second", 0);
        let microsecond = ctor.get_i64(3, "microsecond", 0);
        let tzinfo = ctor.get(4, "tzinfo").unwrap_or_else(py_none);
        let fold = ctor.get_i64(5, "fold", 0);
        if !(0..24).contains(&hour) { return Err(PyError::value_error("hour must be in 0..23")); }
        if !(0..60).contains(&minute) { return Err(PyError::value_error("minute must be in 0..59")); }
        if !(0..60).contains(&second) { return Err(PyError::value_error("second must be in 0..59")); }
        if !(0..1_000_000).contains(&microsecond) { return Err(PyError::value_error("microsecond must be in 0..999999")); }
        if let PyObject::Instance { dict, .. } = &mut *args[0].borrow_mut() {
            dict.insert("hour".to_string(), py_int(hour));
            dict.insert("minute".to_string(), py_int(minute));
            dict.insert("second".to_string(), py_int(second));
            dict.insert("microsecond".to_string(), py_int(microsecond));
            dict.insert("tzinfo".to_string(), tzinfo);
            dict.insert("fold".to_string(), py_int(fold));
        }
        Ok(py_none())
    }));
    type_dict.insert("isoformat".to_string(), bf!("isoformat", |args| Ok(py_str(&time_isoformat(&args[0])))));
    type_dict.insert("__str__".to_string(), bf!("__str__", |args| Ok(py_str(&time_isoformat(&args[0])))));
    type_dict.insert("__repr__".to_string(), bf!("__repr__", |args| {
        Ok(py_str(&format!("datetime.time({}, {}, {})", inst_get_i64(&args[0], "hour"), inst_get_i64(&args[0], "minute"), inst_get_i64(&args[0], "second"))))
    }));
    type_dict.insert("replace".to_string(), bf!("replace", |args| {
        let ctor = CtorArgs::parse(&args[1..]);
        let hour = ctor.get(0, "hour").and_then(|v| v.as_i64()).unwrap_or_else(|| inst_get_i64(&args[0], "hour"));
        let minute = ctor.get(1, "minute").and_then(|v| v.as_i64()).unwrap_or_else(|| inst_get_i64(&args[0], "minute"));
        let second = ctor.get(2, "second").and_then(|v| v.as_i64()).unwrap_or_else(|| inst_get_i64(&args[0], "second"));
        let microsecond = ctor.get(3, "microsecond").and_then(|v| v.as_i64()).unwrap_or_else(|| inst_get_i64(&args[0], "microsecond"));
        let tzinfo = ctor.get(4, "tzinfo").unwrap_or_else(|| inst_get(&args[0], "tzinfo").unwrap_or_else(py_none));
        let fold = ctor.get_i64(5, "fold", 0);
        Ok(make_time(hour, minute, second, microsecond, tzinfo, fold))
    }));
    type_dict.insert("__eq__".to_string(), bf!("__eq__", |args| {
        if instance_type_name(&args[1]) != "time" { return Ok(py_bool(false)); }
        Ok(py_bool(time_tuple_us(&args[0]) == time_tuple_us(&args[1])))
    }));
    type_dict.insert("__lt__".to_string(), bf!("__lt__", |args| {
        if instance_type_name(&args[1]) != "time" { return Err(PyError::type_error("'<' not supported between instances of 'time' and other type")); }
        Ok(py_bool(time_tuple_us(&args[0]) < time_tuple_us(&args[1])))
    }));
    type_dict.insert("__le__".to_string(), bf!("__le__", |args| {
        if instance_type_name(&args[1]) != "time" { return Err(PyError::type_error("'<=' not supported between instances of 'time' and other type")); }
        Ok(py_bool(time_tuple_us(&args[0]) <= time_tuple_us(&args[1])))
    }));
    type_dict.insert("__gt__".to_string(), bf!("__gt__", |args| {
        if instance_type_name(&args[1]) != "time" { return Err(PyError::type_error("'>' not supported between instances of 'time' and other type")); }
        Ok(py_bool(time_tuple_us(&args[0]) > time_tuple_us(&args[1])))
    }));
    type_dict.insert("__ge__".to_string(), bf!("__ge__", |args| {
        if instance_type_name(&args[1]) != "time" { return Err(PyError::type_error("'>=' not supported between instances of 'time' and other type")); }
        Ok(py_bool(time_tuple_us(&args[0]) >= time_tuple_us(&args[1])))
    }));
    type_dict.insert("__hash__".to_string(), bf!("__hash__", |args| Ok(py_int(time_tuple_us(&args[0])))));
    type_dict.insert("utcoffset".to_string(), bf!("utcoffset", |args| {
        let tzinfo = inst_get(&args[0], "tzinfo").unwrap_or_else(py_none);
        match get_utcoffset_seconds(&tzinfo, EPOCH_ORDINAL, 0) {
            Some(s) => Ok(make_timedelta(0, s, 0)),
            None => Ok(py_none()),
        }
    }));
    type_dict.insert("tzname".to_string(), bf!("tzname", |args| {
        let tzinfo = inst_get(&args[0], "tzinfo").unwrap_or_else(py_none);
        match tzname_for(&tzinfo, EPOCH_ORDINAL, 0) {
            Some(s) => Ok(py_str(&s)),
            None => Ok(py_none()),
        }
    }));

    PyObjectRef::new(PyObject::Type { name: "time".to_string(), dict: type_dict, bases: vec![], mro: vec![] })
}

fn make_time(hour: i64, minute: i64, second: i64, microsecond: i64, tzinfo: PyObjectRef, fold: i64) -> PyObjectRef {
    let typ = get_time_type();
    let mut dict = HashMap::new();
    dict.insert("hour".to_string(), py_int(hour));
    dict.insert("minute".to_string(), py_int(minute));
    dict.insert("second".to_string(), py_int(second));
    dict.insert("microsecond".to_string(), py_int(microsecond));
    dict.insert("tzinfo".to_string(), tzinfo);
    dict.insert("fold".to_string(), py_int(fold));
    PyObjectRef::new(PyObject::Instance { typ, dict })
}

fn get_time_type() -> PyObjectRef {
    let existing = TIME_TYPE.with(|c| c.borrow().clone());
    if let Some(t) = existing { return t; }
    let typ = build_time_type();
    TIME_TYPE.with(|c| { *c.borrow_mut() = Some(typ.clone()); });
    typ
}

// ---- datetime ----

fn datetime_ordinal(obj: &PyObjectRef) -> i64 {
    ymd_to_ordinal(inst_get_i64(obj, "year"), inst_get_i64(obj, "month"), inst_get_i64(obj, "day"))
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
    make_datetime(y, mo, d, h as i64, mi as i64, s as i64, us as i64, tzinfo, 0)
}

fn datetime_isoformat(obj: &PyObjectRef, sep: char) -> String {
    let y = inst_get_i64(obj, "year");
    let mo = inst_get_i64(obj, "month");
    let d = inst_get_i64(obj, "day");
    let h = inst_get_i64(obj, "hour");
    let mi = inst_get_i64(obj, "minute");
    let s = inst_get_i64(obj, "second");
    let us = inst_get_i64(obj, "microsecond");
    let mut out = format!("{:04}-{:02}-{:02}{}{:02}:{:02}:{:02}", y, mo, d, sep, h, mi, s);
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
    let (date_part, rest) = match s.find(|c: char| c == 'T' || c == ' ') {
        Some(idx) => (&s[..idx], Some(&s[idx + 1..])),
        None => (s, None),
    };
    let dparts: Vec<&str> = date_part.splitn(3, '-').collect();
    if dparts.len() != 3 { return Err(PyError::value_error("Invalid isoformat string")); }
    let year: i64 = dparts[0].parse().map_err(|_| PyError::value_error("Invalid isoformat string"))?;
    let month: i64 = dparts[1].parse().map_err(|_| PyError::value_error("Invalid isoformat string"))?;
    let day: i64 = dparts[2].parse().map_err(|_| PyError::value_error("Invalid isoformat string"))?;
    let rest = match rest {
        Some(r) => r,
        None => return Ok(make_date(year, month, day)),
    };
    let (time_part, tz_part) = match rest.rfind(['+', '-']) {
        Some(pos) => (&rest[..pos], Some(&rest[pos..])),
        None => (rest, None),
    };
    let tparts: Vec<&str> = time_part.splitn(3, ':').collect();
    let hour: i64 = tparts.first().and_then(|v| v.parse().ok()).unwrap_or(0);
    let minute: i64 = tparts.get(1).and_then(|v| v.parse().ok()).unwrap_or(0);
    let (second, micro): (i64, i64) = match tparts.get(2) {
        Some(sec_str) => match sec_str.find('.') {
            Some(dotpos) => {
                let sec: i64 = sec_str[..dotpos].parse().unwrap_or(0);
                let frac = &sec_str[dotpos + 1..];
                let padded = format!("{:0<6}", frac);
                let us: i64 = padded[..6.min(padded.len())].parse().unwrap_or(0);
                (sec, us)
            }
            None => (sec_str.parse().unwrap_or(0), 0),
        },
        None => (0, 0),
    };
    let tzinfo = match tz_part {
        Some(tz_str) if !tz_str.is_empty() => {
            let sign: i64 = if tz_str.starts_with('-') { -1 } else { 1 };
            let tz_body = &tz_str[1..];
            let tzp: Vec<&str> = tz_body.splitn(2, ':').collect();
            let th: i64 = tzp.first().and_then(|v| v.parse().ok()).unwrap_or(0);
            let tm: i64 = tzp.get(1).and_then(|v| v.parse().ok()).unwrap_or(0);
            make_timezone(sign * (th * 3600 + tm * 60), None)
        }
        _ => py_none(),
    };
    Ok(make_datetime(year, month, day, hour, minute, second, micro, tzinfo, 0))
}

fn build_datetime_type() -> PyObjectRef {
    let mut type_dict: HashMap<String, PyObjectRef> = HashMap::new();
    macro_rules! bf {
        ($name:expr, $f:expr) => {
            PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $f })
        };
    }

    type_dict.insert("__init__".to_string(), bf!("__init__", |args| {
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
        if !(1..=9999).contains(&year) { return Err(PyError::value_error("year out of range")); }
        if !(1..=12).contains(&month) { return Err(PyError::value_error("month must be in 1..12")); }
        if !(1..=days_in_month(year, month)).contains(&day) { return Err(PyError::value_error("day is out of range for month")); }
        if !(0..24).contains(&hour) { return Err(PyError::value_error("hour must be in 0..23")); }
        if !(0..60).contains(&minute) { return Err(PyError::value_error("minute must be in 0..59")); }
        if !(0..60).contains(&second) { return Err(PyError::value_error("second must be in 0..59")); }
        if !(0..1_000_000).contains(&microsecond) { return Err(PyError::value_error("microsecond must be in 0..999999")); }
        if let PyObject::Instance { dict, .. } = &mut *args[0].borrow_mut() {
            dict.insert("year".to_string(), py_int(year));
            dict.insert("month".to_string(), py_int(month));
            dict.insert("day".to_string(), py_int(day));
            dict.insert("hour".to_string(), py_int(hour));
            dict.insert("minute".to_string(), py_int(minute));
            dict.insert("second".to_string(), py_int(second));
            dict.insert("microsecond".to_string(), py_int(microsecond));
            dict.insert("tzinfo".to_string(), tzinfo);
            dict.insert("fold".to_string(), py_int(fold));
        }
        Ok(py_none())
    }));
    type_dict.insert("isoformat".to_string(), bf!("isoformat", |args| {
        let sep = if args.len() > 1 { args[1].str().chars().next().unwrap_or('T') } else { 'T' };
        Ok(py_str(&datetime_isoformat(&args[0], sep)))
    }));
    type_dict.insert("__str__".to_string(), bf!("__str__", |args| Ok(py_str(&datetime_isoformat(&args[0], ' ')))));
    type_dict.insert("__repr__".to_string(), bf!("__repr__", |args| {
        Ok(py_str(&format!(
            "datetime.datetime({}, {}, {}, {}, {}, {})",
            inst_get_i64(&args[0], "year"), inst_get_i64(&args[0], "month"), inst_get_i64(&args[0], "day"),
            inst_get_i64(&args[0], "hour"), inst_get_i64(&args[0], "minute"), inst_get_i64(&args[0], "second"),
        )))
    }));
    type_dict.insert("date".to_string(), bf!("date", |args| Ok(make_date(inst_get_i64(&args[0], "year"), inst_get_i64(&args[0], "month"), inst_get_i64(&args[0], "day")))));
    type_dict.insert("time".to_string(), bf!("time", |args| Ok(make_time(inst_get_i64(&args[0], "hour"), inst_get_i64(&args[0], "minute"), inst_get_i64(&args[0], "second"), inst_get_i64(&args[0], "microsecond"), py_none(), 0))));
    type_dict.insert("timetz".to_string(), bf!("timetz", |args| Ok(make_time(inst_get_i64(&args[0], "hour"), inst_get_i64(&args[0], "minute"), inst_get_i64(&args[0], "second"), inst_get_i64(&args[0], "microsecond"), datetime_tzinfo(&args[0]), 0))));
    type_dict.insert("weekday".to_string(), bf!("weekday", |args| Ok(py_int(weekday_from_ordinal(datetime_ordinal(&args[0]))))));
    type_dict.insert("isoweekday".to_string(), bf!("isoweekday", |args| Ok(py_int(weekday_from_ordinal(datetime_ordinal(&args[0])) + 1))));
    type_dict.insert("toordinal".to_string(), bf!("toordinal", |args| Ok(py_int(datetime_ordinal(&args[0])))));
    type_dict.insert("timestamp".to_string(), bf!("timestamp", |args| {
        let ord = datetime_ordinal(&args[0]);
        let day_us = datetime_day_us(&args[0]);
        let tz = datetime_tzinfo(&args[0]);
        let off = get_utcoffset_seconds(&tz, ord, day_us / 1_000_000).unwrap_or(0);
        let unix_us = (ord - EPOCH_ORDINAL) as i128 * 86_400_000_000 + day_us as i128 - (off as i128) * 1_000_000;
        Ok(py_float(unix_us as f64 / 1_000_000.0))
    }));
    type_dict.insert("utcoffset".to_string(), bf!("utcoffset", |args| {
        let tz = datetime_tzinfo(&args[0]);
        match get_utcoffset_seconds(&tz, datetime_ordinal(&args[0]), datetime_day_us(&args[0]) / 1_000_000) {
            Some(s) => Ok(make_timedelta(0, s, 0)),
            None => Ok(py_none()),
        }
    }));
    type_dict.insert("dst".to_string(), bf!("dst", |args| {
        let tz = datetime_tzinfo(&args[0]);
        if matches!(tz, PyObjectRef::None) { return Ok(py_none()); }
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
    }));
    type_dict.insert("tzname".to_string(), bf!("tzname", |args| {
        let tz = datetime_tzinfo(&args[0]);
        match tzname_for(&tz, datetime_ordinal(&args[0]), datetime_day_us(&args[0]) / 1_000_000) {
            Some(s) => Ok(py_str(&s)),
            None => Ok(py_none()),
        }
    }));
    type_dict.insert("replace".to_string(), bf!("replace", |args| {
        let ctor = CtorArgs::parse(&args[1..]);
        let year = ctor.get(0, "year").and_then(|v| v.as_i64()).unwrap_or_else(|| inst_get_i64(&args[0], "year"));
        let month = ctor.get(1, "month").and_then(|v| v.as_i64()).unwrap_or_else(|| inst_get_i64(&args[0], "month"));
        let day = ctor.get(2, "day").and_then(|v| v.as_i64()).unwrap_or_else(|| inst_get_i64(&args[0], "day"));
        let hour = ctor.get(3, "hour").and_then(|v| v.as_i64()).unwrap_or_else(|| inst_get_i64(&args[0], "hour"));
        let minute = ctor.get(4, "minute").and_then(|v| v.as_i64()).unwrap_or_else(|| inst_get_i64(&args[0], "minute"));
        let second = ctor.get(5, "second").and_then(|v| v.as_i64()).unwrap_or_else(|| inst_get_i64(&args[0], "second"));
        let microsecond = ctor.get(6, "microsecond").and_then(|v| v.as_i64()).unwrap_or_else(|| inst_get_i64(&args[0], "microsecond"));
        let tzinfo = ctor.get(7, "tzinfo").unwrap_or_else(|| datetime_tzinfo(&args[0]));
        let fold = ctor.get_i64(8, "fold", 0);
        Ok(make_datetime(year, month, day, hour, minute, second, microsecond, tzinfo, fold))
    }));
    type_dict.insert("astimezone".to_string(), bf!("astimezone", |args| {
        let new_tz = if args.len() > 1 { args[1].clone() } else { py_none() };
        let total_utc = datetime_total_us_utc(&args[0]);
        let ord = total_utc.div_euclid(86_400_000_000);
        let day_us_utc = total_utc.rem_euclid(86_400_000_000);
        let off = get_utcoffset_seconds(&new_tz, ord as i64, (day_us_utc / 1_000_000) as i64).unwrap_or(0);
        let local_total = total_utc + (off as i128) * 1_000_000;
        Ok(make_datetime_from_total_us(local_total, new_tz))
    }));
    type_dict.insert("strftime".to_string(), bf!("strftime", |args| {
        let fmt = if args.len() > 1 { args[1].str() } else { "%Y-%m-%d %H:%M:%S".to_string() };
        let ord = datetime_ordinal(&args[0]);
        let year = inst_get_i64(&args[0], "year");
        Ok(py_str(&format_strftime(
            &fmt, year, inst_get_i64(&args[0], "month"), inst_get_i64(&args[0], "day"),
            inst_get_i64(&args[0], "hour"), inst_get_i64(&args[0], "minute"), inst_get_i64(&args[0], "second"),
            weekday_from_ordinal(ord), day_of_year(year, ord),
        )))
    }));
    type_dict.insert("__eq__".to_string(), bf!("__eq__", |args| {
        if instance_type_name(&args[1]) != "datetime" { return Ok(py_bool(false)); }
        if datetime_is_aware(&args[0]) != datetime_is_aware(&args[1]) { return Ok(py_bool(false)); }
        Ok(py_bool(datetime_total_us_utc(&args[0]) == datetime_total_us_utc(&args[1])))
    }));
    type_dict.insert("__lt__".to_string(), bf!("__lt__", |args| {
        if instance_type_name(&args[1]) != "datetime" { return Err(PyError::type_error("'<' not supported between instances of 'datetime.datetime' and other type")); }
        if datetime_is_aware(&args[0]) != datetime_is_aware(&args[1]) { return Err(PyError::type_error("can't compare offset-naive and offset-aware datetimes")); }
        Ok(py_bool(datetime_total_us_utc(&args[0]) < datetime_total_us_utc(&args[1])))
    }));
    type_dict.insert("__le__".to_string(), bf!("__le__", |args| {
        if instance_type_name(&args[1]) != "datetime" { return Err(PyError::type_error("'<=' not supported between instances of 'datetime.datetime' and other type")); }
        if datetime_is_aware(&args[0]) != datetime_is_aware(&args[1]) { return Err(PyError::type_error("can't compare offset-naive and offset-aware datetimes")); }
        Ok(py_bool(datetime_total_us_utc(&args[0]) <= datetime_total_us_utc(&args[1])))
    }));
    type_dict.insert("__gt__".to_string(), bf!("__gt__", |args| {
        if instance_type_name(&args[1]) != "datetime" { return Err(PyError::type_error("'>' not supported between instances of 'datetime.datetime' and other type")); }
        if datetime_is_aware(&args[0]) != datetime_is_aware(&args[1]) { return Err(PyError::type_error("can't compare offset-naive and offset-aware datetimes")); }
        Ok(py_bool(datetime_total_us_utc(&args[0]) > datetime_total_us_utc(&args[1])))
    }));
    type_dict.insert("__ge__".to_string(), bf!("__ge__", |args| {
        if instance_type_name(&args[1]) != "datetime" { return Err(PyError::type_error("'>=' not supported between instances of 'datetime.datetime' and other type")); }
        if datetime_is_aware(&args[0]) != datetime_is_aware(&args[1]) { return Err(PyError::type_error("can't compare offset-naive and offset-aware datetimes")); }
        Ok(py_bool(datetime_total_us_utc(&args[0]) >= datetime_total_us_utc(&args[1])))
    }));
    type_dict.insert("__hash__".to_string(), bf!("__hash__", |args| Ok(py_int(datetime_total_us_utc(&args[0]) as i64))));
    type_dict.insert("__add__".to_string(), bf!("__add__", |args| {
        if instance_type_name(&args[1]) != "timedelta" { return Err(PyError::type_error("unsupported operand type(s) for +: 'datetime.datetime' and other type")); }
        let td_us = timedelta_total_us(&args[1]);
        let ord = datetime_ordinal(&args[0]);
        let day_us = datetime_day_us(&args[0]) as i128;
        let total = (ord as i128) * 86_400_000_000 + day_us + td_us;
        Ok(make_datetime_from_total_us(total, datetime_tzinfo(&args[0])))
    }));
    type_dict.insert("__radd__".to_string(), bf!("__radd__", |args| {
        if instance_type_name(&args[1]) != "timedelta" { return Err(PyError::type_error("unsupported operand type(s) for +: 'datetime.datetime' and other type")); }
        let td_us = timedelta_total_us(&args[1]);
        let ord = datetime_ordinal(&args[0]);
        let day_us = datetime_day_us(&args[0]) as i128;
        let total = (ord as i128) * 86_400_000_000 + day_us + td_us;
        Ok(make_datetime_from_total_us(total, datetime_tzinfo(&args[0])))
    }));
    type_dict.insert("__sub__".to_string(), bf!("__sub__", |args| {
        match instance_type_name(&args[1]).as_str() {
            "timedelta" => {
                let td_us = timedelta_total_us(&args[1]);
                let ord = datetime_ordinal(&args[0]);
                let day_us = datetime_day_us(&args[0]) as i128;
                let total = (ord as i128) * 86_400_000_000 + day_us - td_us;
                Ok(make_datetime_from_total_us(total, datetime_tzinfo(&args[0])))
            }
            "datetime" => {
                if datetime_is_aware(&args[0]) != datetime_is_aware(&args[1]) {
                    return Err(PyError::type_error("can't subtract offset-naive and offset-aware datetimes"));
                }
                Ok(make_timedelta_from_us(datetime_total_us_utc(&args[0]) - datetime_total_us_utc(&args[1])))
            }
            _ => Err(PyError::type_error("unsupported operand type(s) for -: 'datetime.datetime' and other type")),
        }
    }));
    type_dict.insert("now".to_string(), bf!("now", |args| {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        let (y, mo, d, h, mi, s, _, _) = epoch_to_ymd(now.as_secs() as i64);
        let us = (now.subsec_nanos() / 1000) as i64;
        let tz = if !args.is_empty() && !matches!(args[0], PyObjectRef::None) { args[0].clone() } else { py_none() };
        if matches!(tz, PyObjectRef::None) {
            Ok(make_datetime(y, mo, d, h, mi, s, us, py_none(), 0))
        } else {
            let naive_ord = ymd_to_ordinal(y, mo, d);
            let day_us = ((h * 3600 + mi * 60 + s) * 1_000_000) + us;
            let off = get_utcoffset_seconds(&tz, naive_ord, day_us / 1_000_000).unwrap_or(0);
            let total = (naive_ord as i128) * 86_400_000_000 + day_us as i128 + (off as i128) * 1_000_000;
            Ok(make_datetime_from_total_us(total, tz))
        }
    }));
    type_dict.insert("utcnow".to_string(), bf!("utcnow", |_args| {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        let (y, mo, d, h, mi, s, _, _) = epoch_to_ymd(now.as_secs() as i64);
        let us = (now.subsec_nanos() / 1000) as i64;
        Ok(make_datetime(y, mo, d, h, mi, s, us, py_none(), 0))
    }));
    type_dict.insert("today".to_string(), bf!("today", |_args| {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        let (y, mo, d, h, mi, s, _, _) = epoch_to_ymd(now.as_secs() as i64);
        Ok(make_datetime(y, mo, d, h, mi, s, 0, py_none(), 0))
    }));
    type_dict.insert("fromtimestamp".to_string(), bf!("fromtimestamp", |args| {
        let ts = if !args.is_empty() { args[0].as_f64().unwrap_or(0.0) } else { 0.0 };
        let tz = if args.len() > 1 && !matches!(args[1], PyObjectRef::None) { args[1].clone() } else { py_none() };
        let secs = ts.floor() as i64;
        let us = ((ts - ts.floor()) * 1_000_000.0).round() as i64;
        if matches!(tz, PyObjectRef::None) {
            let (y, mo, d, h, mi, s, _, _) = epoch_to_ymd(secs);
            Ok(make_datetime(y, mo, d, h, mi, s, us, py_none(), 0))
        } else {
            let off = get_utcoffset_seconds(&tz, EPOCH_ORDINAL + secs.div_euclid(86400), secs.rem_euclid(86400)).unwrap_or(0);
            let (y, mo, d, h, mi, s, _, _) = epoch_to_ymd(secs + off);
            Ok(make_datetime(y, mo, d, h, mi, s, us, tz, 0))
        }
    }));
    type_dict.insert("utcfromtimestamp".to_string(), bf!("utcfromtimestamp", |args| {
        let ts = if !args.is_empty() { args[0].as_f64().unwrap_or(0.0) } else { 0.0 };
        let secs = ts.floor() as i64;
        let us = ((ts - ts.floor()) * 1_000_000.0).round() as i64;
        let (y, mo, d, h, mi, s, _, _) = epoch_to_ymd(secs);
        Ok(make_datetime(y, mo, d, h, mi, s, us, py_none(), 0))
    }));
    type_dict.insert("combine".to_string(), bf!("combine", |args| {
        if args.len() < 2 { return Err(PyError::type_error("combine() requires date and time arguments")); }
        let d = &args[0];
        let t = &args[1];
        let tzinfo = if args.len() > 2 { args[2].clone() } else { inst_get(t, "tzinfo").unwrap_or_else(py_none) };
        Ok(make_datetime(
            inst_get_i64(d, "year"), inst_get_i64(d, "month"), inst_get_i64(d, "day"),
            inst_get_i64(t, "hour"), inst_get_i64(t, "minute"), inst_get_i64(t, "second"), inst_get_i64(t, "microsecond"),
            tzinfo, 0,
        ))
    }));
    type_dict.insert("fromisoformat".to_string(), bf!("fromisoformat", |args| {
        let s = if !args.is_empty() { args[0].str() } else { String::new() };
        parse_datetime_isoformat(&s)
    }));

    PyObjectRef::new(PyObject::Type { name: "datetime".to_string(), dict: type_dict, bases: vec![], mro: vec![] })
}

fn make_datetime(year: i64, month: i64, day: i64, hour: i64, minute: i64, second: i64, microsecond: i64, tzinfo: PyObjectRef, fold: i64) -> PyObjectRef {
    let typ = get_datetime_type();
    let mut dict = HashMap::new();
    dict.insert("year".to_string(), py_int(year));
    dict.insert("month".to_string(), py_int(month));
    dict.insert("day".to_string(), py_int(day));
    dict.insert("hour".to_string(), py_int(hour));
    dict.insert("minute".to_string(), py_int(minute));
    dict.insert("second".to_string(), py_int(second));
    dict.insert("microsecond".to_string(), py_int(microsecond));
    dict.insert("tzinfo".to_string(), tzinfo);
    dict.insert("fold".to_string(), py_int(fold));
    PyObjectRef::new(PyObject::Instance { typ, dict })
}

fn get_datetime_type() -> PyObjectRef {
    let existing = DATETIME_TYPE.with(|c| c.borrow().clone());
    if let Some(t) = existing { return t; }
    let typ = build_datetime_type();
    DATETIME_TYPE.with(|c| { *c.borrow_mut() = Some(typ.clone()); });
    typ
}

// ---- timezone ----

fn build_timezone_type() -> PyObjectRef {
    let mut type_dict: HashMap<String, PyObjectRef> = HashMap::new();
    macro_rules! bf {
        ($name:expr, $f:expr) => {
            PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $f })
        };
    }

    type_dict.insert("__init__".to_string(), bf!("__init__", |args| {
        let ctor = CtorArgs::parse(&args[1..]);
        let offset_seconds = match ctor.get(0, "offset") {
            Some(td) if instance_type_name(&td) == "timedelta" => inst_get_i64(&td, "days") * 86400 + inst_get_i64(&td, "seconds"),
            _ => 0,
        };
        let name = ctor.get(1, "name").map(|v| v.str());
        if let PyObject::Instance { dict, .. } = &mut *args[0].borrow_mut() {
            dict.insert("_offset_seconds".to_string(), py_int(offset_seconds));
            dict.insert("_name".to_string(), name.map(|n| py_str(&n)).unwrap_or_else(py_none));
        }
        Ok(py_none())
    }));
    type_dict.insert("utcoffset".to_string(), bf!("utcoffset", |args| Ok(make_timedelta(0, inst_get_i64(&args[0], "_offset_seconds"), 0))));
    type_dict.insert("dst".to_string(), bf!("dst", |_args| Ok(py_none())));
    type_dict.insert("tzname".to_string(), bf!("tzname", |args| {
        match inst_get(&args[0], "_name") {
            Some(v) if !matches!(v, PyObjectRef::None) => Ok(v),
            _ => Ok(py_str(&format_utc_offset_name(inst_get_i64(&args[0], "_offset_seconds")))),
        }
    }));
    type_dict.insert("__eq__".to_string(), bf!("__eq__", |args| {
        if instance_type_name(&args[1]) != "timezone" { return Ok(py_bool(false)); }
        Ok(py_bool(inst_get_i64(&args[0], "_offset_seconds") == inst_get_i64(&args[1], "_offset_seconds")))
    }));
    type_dict.insert("__hash__".to_string(), bf!("__hash__", |args| Ok(py_int(inst_get_i64(&args[0], "_offset_seconds")))));
    type_dict.insert("__repr__".to_string(), bf!("__repr__", |args| {
        let off = inst_get_i64(&args[0], "_offset_seconds");
        if off == 0 { Ok(py_str("datetime.timezone.utc")) } else { Ok(py_str(&format!("datetime.timezone(datetime.timedelta(seconds={}))", off))) }
    }));
    type_dict.insert("__str__".to_string(), bf!("__str__", |args| {
        match inst_get(&args[0], "_name") {
            Some(v) if !matches!(v, PyObjectRef::None) => Ok(v),
            _ => Ok(py_str(&format_utc_offset_name(inst_get_i64(&args[0], "_offset_seconds")))),
        }
    }));

    PyObjectRef::new(PyObject::Type { name: "timezone".to_string(), dict: type_dict, bases: vec![], mro: vec![] })
}

fn make_timezone_with_type(typ: PyObjectRef, offset_seconds: i64, name: Option<String>) -> PyObjectRef {
    let mut dict = HashMap::new();
    dict.insert("_offset_seconds".to_string(), py_int(offset_seconds));
    dict.insert("_name".to_string(), name.map(|n| py_str(&n)).unwrap_or_else(py_none));
    PyObjectRef::new(PyObject::Instance { typ, dict })
}

fn make_timezone(offset_seconds: i64, name: Option<String>) -> PyObjectRef {
    make_timezone_with_type(get_timezone_type(), offset_seconds, name)
}

fn get_timezone_type() -> PyObjectRef {
    let existing = TIMEZONE_TYPE.with(|c| c.borrow().clone());
    if let Some(t) = existing { return t; }
    let typ = build_timezone_type();
    let utc_inst = make_timezone_with_type(typ.clone(), 0, None);
    if let PyObject::Type { dict, .. } = &mut *typ.borrow_mut() {
        dict.insert("utc".to_string(), utc_inst);
    }
    TIMEZONE_TYPE.with(|c| { *c.borrow_mut() = Some(typ.clone()); });
    typ
}

// ---- zoneinfo.ZoneInfo ----

fn build_zoneinfo_type() -> PyObjectRef {
    let mut type_dict: HashMap<String, PyObjectRef> = HashMap::new();
    macro_rules! bf {
        ($name:expr, $f:expr) => {
            PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $f })
        };
    }

    type_dict.insert("__init__".to_string(), bf!("__init__", |args| {
        if args.len() < 2 { return Err(PyError::type_error("ZoneInfo() missing key argument")); }
        let key = args[1].str();
        if load_tz(&key).is_none() {
            return Err(PyError::key_error(format!("No time zone found with key {}", key)));
        }
        if let PyObject::Instance { dict, .. } = &mut *args[0].borrow_mut() {
            dict.insert("key".to_string(), py_str(&key));
        }
        Ok(py_none())
    }));
    type_dict.insert("utcoffset".to_string(), bf!("utcoffset", |args| {
        if args.len() < 2 { return Err(PyError::type_error("utcoffset() missing datetime argument")); }
        let key = inst_get(&args[0], "key").map(|v| v.str()).unwrap_or_default();
        let tz = load_tz(&key).ok_or_else(|| PyError::runtime_error("zone data not found"))?;
        let dt = &args[1];
        let ord = ymd_to_ordinal(inst_get_i64(dt, "year"), inst_get_i64(dt, "month"), inst_get_i64(dt, "day"));
        let day_secs = inst_get_i64(dt, "hour") * 3600 + inst_get_i64(dt, "minute") * 60 + inst_get_i64(dt, "second");
        let unix_instant = (ord - EPOCH_ORDINAL) * 86400 + day_secs;
        let (off, _, _) = tz_offset_for_instant(&tz, unix_instant);
        Ok(make_timedelta(0, off as i64, 0))
    }));
    type_dict.insert("dst".to_string(), bf!("dst", |args| {
        if args.len() < 2 { return Ok(py_none()); }
        let key = inst_get(&args[0], "key").map(|v| v.str()).unwrap_or_default();
        let tz = load_tz(&key).ok_or_else(|| PyError::runtime_error("zone data not found"))?;
        let dt = &args[1];
        let ord = ymd_to_ordinal(inst_get_i64(dt, "year"), inst_get_i64(dt, "month"), inst_get_i64(dt, "day"));
        let day_secs = inst_get_i64(dt, "hour") * 3600 + inst_get_i64(dt, "minute") * 60 + inst_get_i64(dt, "second");
        let unix_instant = (ord - EPOCH_ORDINAL) * 86400 + day_secs;
        let (_, isdst, _) = tz_offset_for_instant(&tz, unix_instant);
        Ok(make_timedelta(0, if isdst { 3600 } else { 0 }, 0))
    }));
    type_dict.insert("tzname".to_string(), bf!("tzname", |args| {
        if args.len() < 2 { return Ok(py_none()); }
        let key = inst_get(&args[0], "key").map(|v| v.str()).unwrap_or_default();
        let tz = load_tz(&key).ok_or_else(|| PyError::runtime_error("zone data not found"))?;
        let dt = &args[1];
        let ord = ymd_to_ordinal(inst_get_i64(dt, "year"), inst_get_i64(dt, "month"), inst_get_i64(dt, "day"));
        let day_secs = inst_get_i64(dt, "hour") * 3600 + inst_get_i64(dt, "minute") * 60 + inst_get_i64(dt, "second");
        let unix_instant = (ord - EPOCH_ORDINAL) * 86400 + day_secs;
        let (_, _, name) = tz_offset_for_instant(&tz, unix_instant);
        Ok(py_str(&name))
    }));
    type_dict.insert("__repr__".to_string(), bf!("__repr__", |args| {
        let key = inst_get(&args[0], "key").map(|v| v.str()).unwrap_or_default();
        Ok(py_str(&format!("zoneinfo.ZoneInfo(key='{}')", key)))
    }));
    type_dict.insert("__str__".to_string(), bf!("__str__", |args| Ok(inst_get(&args[0], "key").unwrap_or_else(|| py_str("")))));
    type_dict.insert("__eq__".to_string(), bf!("__eq__", |args| {
        if instance_type_name(&args[1]) != "ZoneInfo" { return Ok(py_bool(false)); }
        let a = inst_get(&args[0], "key").map(|v| v.str()).unwrap_or_default();
        let b = inst_get(&args[1], "key").map(|v| v.str()).unwrap_or_default();
        Ok(py_bool(a == b))
    }));
    type_dict.insert("__hash__".to_string(), bf!("__hash__", |args| {
        let key = inst_get(&args[0], "key").map(|v| v.str()).unwrap_or_default();
        builtin_hash(&[py_str(&key)])
    }));

    PyObjectRef::new(PyObject::Type { name: "ZoneInfo".to_string(), dict: type_dict, bases: vec![], mro: vec![] })
}

fn get_zoneinfo_type() -> PyObjectRef {
    let existing = ZONEINFO_TYPE.with(|c| c.borrow().clone());
    if let Some(t) = existing { return t; }
    let typ = build_zoneinfo_type();
    ZONEINFO_TYPE.with(|c| { *c.borrow_mut() = Some(typ.clone()); });
    typ
}

pub fn create_datetime_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    d.insert("date".to_string(), get_date_type());
    d.insert("time".to_string(), get_time_type());
    d.insert("datetime".to_string(), get_datetime_type());
    d.insert("timedelta".to_string(), get_timedelta_type());
    let timezone_type = get_timezone_type();
    let utc_singleton = if let PyObject::Type { dict, .. } = &*timezone_type.borrow() {
        dict.get("utc").cloned().unwrap_or_else(|| make_timezone(0, None))
    } else {
        make_timezone(0, None)
    };
    d.insert("timezone".to_string(), timezone_type);
    // `datetime.UTC` (3.11+) is the exact same object as `timezone.utc`.
    d.insert("UTC".to_string(), utc_singleton);
    d.insert("tzinfo".to_string(), get_tzinfo_type());
    d.insert("MINYEAR".to_string(), py_int(1));
    d.insert("MAXYEAR".to_string(), py_int(9999));
    d
}

pub fn create_zoneinfo_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    d.insert("ZoneInfo".to_string(), get_zoneinfo_type());
    d.insert("available_timezones".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
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
                    if name.starts_with('.') { continue; }
                    let skip = matches!(name.as_str(), "posix" | "right" | "posixrules" | "Factory"
                        | "iso3166.tab" | "zone.tab" | "zone1970.tab" | "tzdata.zi" | "leapseconds" | "leap-seconds.list");
                    if skip { continue; }
                    let path = entry.path();
                    let rel = if prefix.is_empty() { name.clone() } else { format!("{}/{}", prefix, name) };
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
    }));
    d
}
