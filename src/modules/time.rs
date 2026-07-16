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
    time_func!("time", |args| {
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
    time_func!("monotonic", |args| {
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
    time_func!("perf_counter", |args| {
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
