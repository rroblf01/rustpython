use crate::object::*;
use std::collections::HashMap;

fn day_of_week(y: i64, m: i64, d: i64) -> usize {
    let (y, m) = if m < 3 { (y - 1, m + 12) } else { (y, m) };
    let k = y % 100;
    let j = y / 100;
    let h = (d + (13 * (m + 1)) / 5 + k + k / 4 + j / 4 + 5 * j) % 7;
    // h: 0=Saturday, 1=Sunday, 2=Monday, ... -> convert to Monday=0..Sunday=6
    ((h + 5) % 7) as usize
}

fn rfc2822_date(y: i64, mo: i64, d: i64, h: i64, mi: i64, s: i64) -> String {
    const DAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let wd = DAYS[day_of_week(y, mo, d)];
    let mon = MONTHS[((mo - 1).clamp(0, 11)) as usize];
    format!(
        "{}, {:02} {} {:04} {:02}:{:02}:{:02} +0000",
        wd, d, mon, y, h, mi, s
    )
}

fn unix_secs_to_ymdhms(secs: i64) -> (i64, i64, i64, i64, i64, i64) {
    let days = secs.div_euclid(86400);
    let day_secs = secs.rem_euclid(86400);
    let hours = day_secs / 3600;
    let minutes = (day_secs / 60) % 60;
    let seconds = day_secs % 60;
    let mut y = 1970i64;
    let mut remaining = days;
    loop {
        let year_days = if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) {
            366
        } else {
            365
        };
        if remaining >= year_days {
            remaining -= year_days;
            y += 1;
        } else if remaining < 0 {
            y -= 1;
            let yd = if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) {
                366
            } else {
                365
            };
            remaining += yd;
        } else {
            break;
        }
    }
    let is_leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let month_days = [
        31,
        if is_leap { 29 } else { 28 },
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
    let mut m = 1i64;
    for days_in_month in &month_days {
        if remaining < *days_in_month {
            break;
        }
        remaining -= days_in_month;
        m += 1;
    }
    (y, m, remaining + 1, hours, minutes, seconds)
}

pub fn create_email_utils_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! eu_func {
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
    // formatdate(timeval=None, localtime=False, usegmt=False) -> string
    eu_func!("formatdate", |args| {
        let secs = if !args.is_empty() && !matches!(&*args[0].borrow(), PyObject::None) {
            args[0].as_f64().unwrap_or(0.0) as i64
        } else {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64
        };
        let (y, mo, d, h, mi, s) = unix_secs_to_ymdhms(secs);
        Ok(py_str(&rfc2822_date(y, mo, d, h, mi, s)))
    });
    // format_datetime(dt, usegmt=False) -> string — reads year/month/day/
    // hour/minute/second attributes off the given datetime-like object.
    eu_func!("format_datetime", |args| {
        if args.is_empty() {
            return Err(PyError::type_error(
                "format_datetime() missing required argument",
            ));
        }
        let get = |name: &str, default: i64| -> i64 {
            args[0]
                .borrow()
                .get_attribute(name)
                .ok()
                .and_then(|v| v.as_i64())
                .unwrap_or(default)
        };
        let y = get("year", 1970);
        let mo = get("month", 1);
        let d = get("day", 1);
        let h = get("hour", 0);
        let mi = get("minute", 0);
        let s = get("second", 0);
        Ok(py_str(&rfc2822_date(y, mo, d, h, mi, s)))
    });
    d
}
