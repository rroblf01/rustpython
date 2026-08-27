use crate::object::*;
use std::collections::HashMap;

pub fn create_calendar_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! cal_func {
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

    // Add constants to module.
    // `month_name`/`month_abbr` are 1-INDEXED in real CPython (`[0]` is a
    // deliberate empty-string placeholder, `[1]` = "January" .. `[12]` =
    // "December") — matching every other month-numbering convention in
    // Python (`date.month`, `time.tm_mon`, `strftime("%m")`, all 1-12).
    // Missing the `[0]` placeholder here meant `calendar.month_name[12]`
    // (December, the extremely common `month_name[some_real_month_number]`
    // idiom) actually returned November — an off-by-one silently giving
    // the WRONG month name for every single lookup, not a crash. Real
    // trigger: CPython's own `_strptime.py`, `[calendar.month_abbr[i] for i
    // in range(13)]` (deliberately ranging through 13 to include the
    // placeholder) raising `IndexError` outright once vendored, since the
    // 12-element list had no index 12 at all.
    d.insert_str(
        "month_name",
        py_list(vec![
            py_str(""),
            py_str("January"),
            py_str("February"),
            py_str("March"),
            py_str("April"),
            py_str("May"),
            py_str("June"),
            py_str("July"),
            py_str("August"),
            py_str("September"),
            py_str("October"),
            py_str("November"),
            py_str("December"),
        ]),
    );
    d.insert_str(
        "month_abbr",
        py_list(vec![
            py_str(""),
            py_str("Jan"),
            py_str("Feb"),
            py_str("Mar"),
            py_str("Apr"),
            py_str("May"),
            py_str("Jun"),
            py_str("Jul"),
            py_str("Aug"),
            py_str("Sep"),
            py_str("Oct"),
            py_str("Nov"),
            py_str("Dec"),
        ]),
    );
    d.insert_str(
        "day_name",
        py_list(vec![
            py_str("Monday"),
            py_str("Tuesday"),
            py_str("Wednesday"),
            py_str("Thursday"),
            py_str("Friday"),
            py_str("Saturday"),
            py_str("Sunday"),
        ]),
    );
    d.insert_str(
        "day_abbr",
        py_list(vec![
            py_str("Mon"),
            py_str("Tue"),
            py_str("Wed"),
            py_str("Thu"),
            py_str("Fri"),
            py_str("Sat"),
            py_str("Sun"),
        ]),
    );
    // Weekday constants (0=Monday..6=Sunday, matching `calendar.weekday()`'s
    // own return convention) — were missing entirely.
    d.insert_str("MONDAY", py_int(0));
    d.insert_str("TUESDAY", py_int(1));
    d.insert_str("WEDNESDAY", py_int(2));
    d.insert_str("THURSDAY", py_int(3));
    d.insert_str("FRIDAY", py_int(4));
    d.insert_str("SATURDAY", py_int(5));
    d.insert_str("SUNDAY", py_int(6));

    // Calendar helper functions (inner fn items are not captured by closures)
    fn is_leap(y: i64) -> bool {
        y % 4 == 0 && (y % 100 != 0 || y % 400 == 0)
    }
    fn month_days(y: i64, m: i64) -> i64 {
        match m {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 => {
                if is_leap(y) {
                    29
                } else {
                    28
                }
            }
            _ => 0,
        }
    }
    // Tomohiko Sakamoto's weekday algorithm: returns 0=Sunday, 1=Monday, ..., 6=Saturday
    fn weekday(y: i64, m: i64, d: i64) -> i64 {
        let t = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
        let y = if m < 3 { y - 1 } else { y };
        (y + y / 4 - y / 100 + y / 400 + t[m as usize - 1] + d) % 7
    }
    // First weekday of month: 0=Monday, 6=Sunday
    fn first_weekday(y: i64, m: i64) -> i64 {
        (weekday(y, m, 1) + 6) % 7
    }

    const MONTH_NAMES: [&str; 12] = [
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

    // ---- HTMLCalendar factory ----
    cal_func!("HTMLCalendar", |args| {
        let _ = args;

        const HTML_DAY_CLASS: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];

        // formatmonth method
        let mut type_dict = HashMap::new();
        type_dict.insert_str("formatmonth", PyObjectRef::new(PyObject::BuiltinFunction {
            name: "formatmonth".to_string(),
            func: |args| {
                if args.len() < 3 {
                    return Err(PyError::type_error("formatmonth() missing required arguments (self, year, month)"));
                }
                let y = args[1].as_i64().ok_or_else(|| PyError::type_error("year must be int"))?;
                let m = args[2].as_i64().ok_or_else(|| PyError::type_error("month must be int"))?;
                if m < 1 || m > 12 {
                    return Err(PyError::type_error("month must be in 1..12"));
                }

                let dim = month_days(y, m);
                let fd = first_weekday(y, m);

                let mut html = String::new();
                html.push_str("<table border=\"0\" cellpadding=\"0\" cellspacing=\"0\" class=\"month\">\n");
                html.push_str(&format!(
                    "<tr><th colspan=\"7\" class=\"month\">{} {}</th></tr>\n",
                    MONTH_NAMES[(m - 1) as usize], y
                ));
                html.push_str("<tr><th class=\"mon\">Mon</th><th class=\"tue\">Tue</th><th class=\"wed\">Wed</th>");
                html.push_str("<th class=\"thu\">Thu</th><th class=\"fri\">Fri</th><th class=\"sat\">Sat</th><th class=\"sun\">Sun</th></tr>\n");

                html.push_str("<tr>\n");
                for _ in 0..fd {
                    html.push_str("<td class=\"noday\">&nbsp;</td>");
                }
                for day in 1..=dim {
                    let wd = ((fd + day - 1) % 7) as usize;
                    html.push_str(&format!("<td class=\"{}\">{}</td>", HTML_DAY_CLASS[wd], day));
                    if (fd + day) % 7 == 0 && day != dim {
                        html.push_str("</tr>\n<tr>\n");
                    }
                }
                let remaining = (7 - (fd + dim) % 7) % 7;
                for _ in 0..remaining {
                    html.push_str("<td class=\"noday\">&nbsp;</td>");
                }
                html.push_str("</tr>\n</table>\n");
                Ok(py_str(&html))
            },
        }));

        // formatyear method
        type_dict.insert_str("formatyear", PyObjectRef::new(PyObject::BuiltinFunction {
            name: "formatyear".to_string(),
            func: |args| {
                if args.len() < 2 {
                    return Err(PyError::type_error("formatyear() missing required arguments (self, year)"));
                }
                let y = args[1].as_i64().ok_or_else(|| PyError::type_error("year must be int"))?;

                let mut html = String::new();
                html.push_str(&format!("<table border=\"0\" cellpadding=\"0\" cellspacing=\"0\" class=\"year\">\n"));
                html.push_str(&format!("<tr><th colspan=\"3\" class=\"year\">{}</th></tr>\n", y));

                for q in 0..4 {
                    html.push_str("<tr>\n");
                    for m_idx in 0..3 {
                        let m = q * 3 + m_idx + 1;
                        let dim = month_days(y, m);
                        let fd = first_weekday(y, m);

                        html.push_str("<td>\n<table border=\"0\" cellpadding=\"0\" cellspacing=\"0\" class=\"month\">\n");
                        html.push_str(&format!(
                            "<tr><th colspan=\"7\" class=\"month\">{} {}</th></tr>\n",
                            MONTH_NAMES[(m - 1) as usize], y
                        ));
                        html.push_str("<tr><th class=\"mon\">Mon</th><th class=\"tue\">Tue</th><th class=\"wed\">Wed</th>");
                        html.push_str("<th class=\"thu\">Thu</th><th class=\"fri\">Fri</th><th class=\"sat\">Sat</th><th class=\"sun\">Sun</th></tr>\n");

                        html.push_str("<tr>\n");
                        for _ in 0..fd {
                            html.push_str("<td class=\"noday\">&nbsp;</td>");
                        }
                        for day in 1..=dim {
                            let wd = ((fd + day - 1) % 7) as usize;
                            html.push_str(&format!("<td class=\"{}\">{}</td>", HTML_DAY_CLASS[wd], day));
                            if (fd + day) % 7 == 0 && day != dim {
                                html.push_str("</tr>\n<tr>\n");
                            }
                        }
                        let remaining = (7 - (fd + dim) % 7) % 7;
                        for _ in 0..remaining {
                            html.push_str("<td class=\"noday\">&nbsp;</td>");
                        }
                        html.push_str("</tr>\n</table>\n</td>\n");
                        if m_idx < 2 {
                            html.push_str("<td>&nbsp;</td>\n");
                        }
                    }
                    html.push_str("</tr>\n");
                }
                html.push_str("</table>\n");
                Ok(py_str(&html))
            },
        }));

        Ok(PyObjectRef::new(PyObject::Instance {
            typ: PyObjectRef::new(PyObject::Type {
                name: "HTMLCalendar".to_string(),
                dict: Box::new(str_map_to_typedict(type_dict)),
                bases: vec![],
                mro: vec![],
            }),
            dict: AttrMap::new(),
        }))
    });

    // ---- TextCalendar factory ----
    cal_func!("TextCalendar", |args| {
        let _ = args;
        let mut type_dict = HashMap::new();
        type_dict.insert_str(
            "formatmonth",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "formatmonth".to_string(),
                func: |args| {
                    if args.len() < 3 {
                        return Err(PyError::type_error(
                            "formatmonth() missing required arguments (self, year, month)",
                        ));
                    }
                    let y = match args[1].as_i64() {
                        Some(i) => i,
                        None => return Err(PyError::type_error("year must be int")),
                    };
                    let m = match args[2].as_i64() {
                        Some(i) => i,
                        None => return Err(PyError::type_error("month must be int")),
                    };
                    if m < 1 || m > 12 {
                        return Err(PyError::type_error("month must be in 1..12"));
                    }
                    let dim = month_days(y, m);
                    let fd = first_weekday(y, m);
                    let mut lines = Vec::new();
                    lines.push(format!(
                        "{:>20}",
                        format!("{} {}", MONTH_NAMES[(m - 1) as usize], y)
                    ));
                    lines.push("Mo Tu We Th Fr Sa Su".to_string());
                    let mut week: Vec<String> = Vec::new();
                    for _ in 0..fd {
                        week.push("  ".to_string());
                    }
                    for day in 1..=dim {
                        week.push(format!("{:2}", day));
                        if week.len() == 7 {
                            lines.push(week.join(" "));
                            week.clear();
                        }
                    }
                    if !week.is_empty() {
                        while week.len() < 7 {
                            week.push("  ".to_string());
                        }
                        lines.push(week.join(" "));
                    }
                    Ok(py_str(&lines.join("\n")))
                },
            }),
        );
        Ok(PyObjectRef::new(PyObject::Instance {
            typ: PyObjectRef::new(PyObject::Type {
                name: "TextCalendar".to_string(),
                dict: Box::new(str_map_to_typedict(type_dict)),
                bases: vec![],
                mro: vec![],
            }),
            dict: AttrMap::new(),
        }))
    });

    // ---- Module-level calendar functions ----
    // `calendar.timegm(tuple)` — the inverse of `time.gmtime()`: given a
    // struct_time-shaped tuple (or any 6+-element sequence with year/month/
    // day/hour/min/sec in that order), return Unix seconds treating it as
    // UTC. Missing entirely — real trigger: CPython's own `http/cookiejar.py`
    // (`from calendar import timegm`), needed to convert a parsed
    // `Expires=` cookie header back into a comparable timestamp. Accepts
    // both a real `time.struct_time` (attribute-accessible, see
    // `modules/time.rs`) and a plain tuple, matching real `timegm`'s own
    // "any sequence" acceptance.
    cal_func!("timegm", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("timegm() missing required argument"));
        }
        let get = |i: usize, field: &str| -> i64 {
            match &*args[0].borrow() {
                PyObject::Instance { dict, .. } => {
                    dict.get(field).and_then(|v| v.as_i64()).unwrap_or(0)
                }
                PyObject::Tuple(items) | PyObject::List(items) => {
                    items.get(i).and_then(|v| v.as_i64()).unwrap_or(0)
                }
                _ => 0,
            }
        };
        let year = get(0, "tm_year");
        let month = get(1, "tm_mon");
        let mday = get(2, "tm_mday");
        let hour = get(3, "tm_hour");
        let minute = get(4, "tm_min");
        let second = get(5, "tm_sec");
        // Howard Hinnant civil-days-from-epoch algorithm (same one used by
        // `modules/time.rs`'s `civil_to_days`/`epoch_to_ymd`, duplicated
        // here rather than made cross-module-public since `calendar` and
        // `time` are populated by two separate, independent dict-builder
        // functions with no shared internal-helpers module).
        let y = if month <= 2 { year - 1 } else { year };
        let era = if y >= 0 { y } else { y - 399 } / 400;
        let yoe = y - era * 400;
        let mp = if month > 2 { month - 3 } else { month + 9 };
        let doy = (153 * mp + 2) / 5 + mday - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        let days = era * 146097 + doe - 719468;
        Ok(py_int(days * 86400 + hour * 3600 + minute * 60 + second))
    });

    cal_func!("isleap", |args| {
        if args.len() < 1 {
            return Err(PyError::type_error(
                "isleap() missing required argument (year)",
            ));
        }
        let year = args[0]
            .as_i64()
            .ok_or_else(|| PyError::type_error("year must be integer"))?;
        Ok(py_bool(is_leap(year)))
    });

    cal_func!("weekday", |args| {
        if args.len() < 3 {
            return Err(PyError::type_error(
                "weekday() requires 3 arguments (year, month, day)",
            ));
        }
        let y = args[0]
            .as_i64()
            .ok_or_else(|| PyError::type_error("year must be integer"))?;
        let m = args[1]
            .as_i64()
            .ok_or_else(|| PyError::type_error("month must be integer"))?;
        let d = args[2]
            .as_i64()
            .ok_or_else(|| PyError::type_error("day must be integer"))?;
        // weekday returns 0=Monday, 6=Sunday
        let wd = (weekday(y, m, d) + 6) % 7;
        Ok(py_int(wd))
    });

    cal_func!("monthrange", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error(
                "monthrange() requires 2 arguments (year, month)",
            ));
        }
        let y = args[0]
            .as_i64()
            .ok_or_else(|| PyError::type_error("year must be integer"))?;
        let m = args[1]
            .as_i64()
            .ok_or_else(|| PyError::type_error("month must be integer"))?;
        if m < 1 || m > 12 {
            return Err(PyError::type_error("month must be in 1..12"));
        }
        let fd = first_weekday(y, m);
        let ndays = month_days(y, m);
        Ok(py_tuple(vec![py_int(fd), py_int(ndays)]))
    });

    cal_func!("monthcalendar", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error(
                "monthcalendar() requires 2 arguments (year, month)",
            ));
        }
        let y = args[0]
            .as_i64()
            .ok_or_else(|| PyError::type_error("year must be integer"))?;
        let m = args[1]
            .as_i64()
            .ok_or_else(|| PyError::type_error("month must be integer"))?;
        if m < 1 || m > 12 {
            return Err(PyError::type_error("month must be in 1..12"));
        }
        let dim = month_days(y, m);
        let fd = first_weekday(y, m);
        let mut weeks: Vec<PyObjectRef> = Vec::new();
        let mut week: Vec<PyObjectRef> = Vec::new();
        for _ in 0..fd {
            week.push(py_int(0));
        }
        for day in 1..=dim {
            week.push(py_int(day));
            if week.len() == 7 {
                weeks.push(py_list(week.clone()));
                week.clear();
            }
        }
        if !week.is_empty() {
            while week.len() < 7 {
                week.push(py_int(0));
            }
            weeks.push(py_list(week));
        }
        Ok(py_list(weeks))
    });

    cal_func!("prmonth", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error(
                "prmonth() requires 2 arguments (year, month)",
            ));
        }
        let y = args[0]
            .as_i64()
            .ok_or_else(|| PyError::type_error("year must be integer"))?;
        let m = args[1]
            .as_i64()
            .ok_or_else(|| PyError::type_error("month must be integer"))?;
        if m < 1 || m > 12 {
            return Err(PyError::type_error("month must be in 1..12"));
        }
        // Simplified text print
        println!("     {} {}", MONTH_NAMES[(m - 1) as usize], y);
        println!("Mo Tu We Th Fr Sa Su");
        let dim = month_days(y, m);
        let fd = first_weekday(y, m);
        for _ in 0..fd {
            print!("   ");
        }
        for day in 1..=dim {
            print!("{:2} ", day);
            if (fd + day) % 7 == 0 {
                println!();
            }
        }
        println!();
        Ok(py_none())
    });

    // `calendar.__all__` — same fix, same reason, as `operator.__all__`
    // (`core.rs`) — missing entirely, breaking the module's own
    // `test___all__` sanity check at collection time.
    let all_names: Vec<PyObjectRef> = d
        .keys()
        .filter(|k| !k.starts_with('_'))
        .map(|k| py_str(k))
        .collect();
    d.insert_str("__all__", py_list(all_names));

    d
}

// ── Native _random module (C extension stub for CPython's random.py) ──────
