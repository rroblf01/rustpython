use crate::object::*;
use std::collections::HashMap;

// ---- email module ----

fn email_message_getitem(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Err(PyError::type_error(
            "__getitem__() takes at least 2 arguments (self, key)",
        ));
    }
    let key = args[1].str();
    let inst = args[0].borrow();
    if let PyObject::Instance { dict, .. } = &*inst {
        let header_key = format!("_header_{}", key);
        match dict.get(&header_key) {
            Some(val) => Ok(val.clone()),
            None => Ok(py_none()),
        }
    } else {
        Err(PyError::type_error("EmailMessage instance required"))
    }
}

fn email_message_setitem(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 3 {
        return Err(PyError::type_error(
            "__setitem__() takes at least 3 arguments (self, key, value)",
        ));
    }
    let key = args[1].str();
    let value = args[2].str();
    let mut inst = args[0].borrow_mut();
    if let PyObject::Instance { dict, .. } = &mut *inst {
        let header_key = format!("_header_{}", key);
        dict.insert(header_key, py_str(&value));
    }
    Ok(py_none())
}

fn email_message_set_content(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Err(PyError::type_error(
            "set_content() takes at least 1 argument (text)",
        ));
    }
    let text = args[1].str();
    let mut inst = args[0].borrow_mut();
    if let PyObject::Instance { dict, .. } = &mut *inst {
        dict.insert_str("_content", py_str(&text));
        dict.insert_str("_content_type", py_str("text/plain"));
    }
    Ok(py_none())
}

fn email_message_as_string(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error(
            "as_string() takes at least 1 argument (self)",
        ));
    }
    let inst = args[0].borrow();
    if let PyObject::Instance { dict, .. } = &*inst {
        // Collect headers
        let mut headers: Vec<(String, String)> = Vec::new();
        for (k, v) in dict.iter() {
            if let Some(header_name) = k.strip_prefix("_header_") {
                headers.push((header_name.to_string(), v.str()));
            }
        }
        // Sort known headers first for readability
        let priority = |name: &str| -> usize {
            match name {
                "From" => 0,
                "To" => 1,
                "Subject" => 2,
                _ => 3,
            }
        };
        headers.sort_by_key(|(k, _)| priority(k));

        let content = dict
            .get_str("_content")
            .map(|v| v.str())
            .unwrap_or_default();

        let mut result = String::new();
        for (name, value) in &headers {
            result.push_str(&format!("{}: {}\r\n", name, value));
        }
        result.push_str("\r\n");
        result.push_str(&content);

        Ok(py_str(&result))
    } else {
        Err(PyError::type_error("EmailMessage instance required"))
    }
}

fn email_message_repr(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error(
            "__repr__() takes at least 1 argument (self)",
        ));
    }
    let inst = args[0].borrow();
    if let PyObject::Instance { dict, .. } = &*inst {
        let subject = dict
            .get_str("_header_Subject")
            .map(|v| v.str())
            .unwrap_or_default();
        let from_addr = dict
            .get_str("_header_From")
            .map(|v| v.str())
            .unwrap_or_default();
        let to_addr = dict
            .get_str("_header_To")
            .map(|v| v.str())
            .unwrap_or_default();
        Ok(py_str(&format!(
            "<EmailMessage: From: {}, To: {}, Subject: {}>",
            from_addr, to_addr, subject
        )))
    } else {
        Err(PyError::type_error("EmailMessage instance required"))
    }
}

fn email_message_constructor(_args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    // Create the EmailMessage type
    let mut type_dict = HashMap::new();
    type_dict.insert_str(
        "__getitem__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__getitem__".to_string(),
            func: email_message_getitem,
        }),
    );
    type_dict.insert_str(
        "__setitem__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__setitem__".to_string(),
            func: email_message_setitem,
        }),
    );
    type_dict.insert_str(
        "__repr__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__repr__".to_string(),
            func: email_message_repr,
        }),
    );
    type_dict.insert_str(
        "set_content",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "set_content".to_string(),
            func: email_message_set_content,
        }),
    );
    type_dict.insert_str(
        "as_string",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "as_string".to_string(),
            func: email_message_as_string,
        }),
    );

    let email_type = PyObjectRef::new(PyObject::Type {
        name: "EmailMessage".to_string(),
        dict: Box::new(str_map_to_typedict(type_dict)),
        bases: vec![],
        mro: vec![],
    });

    // Create instance with empty dict
    let instance = PyObjectRef::new(PyObject::Instance {
        typ: email_type,
        dict: AttrMap::new(),
    });

    Ok(instance)
}

pub fn create_email_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! email_func {
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

    // EmailMessage class constructor (callable)
    d.insert_str(
        "EmailMessage",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "EmailMessage".to_string(),
            func: email_message_constructor,
        }),
    );

    // MIMEText is in email.mime.text, but we provide a stub here for convenience
    email_func!("MIMEText", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("MIMEText() missing required argument"));
        }
        let body = args[0].str();
        let subtype = if args.len() > 1 {
            args[1].str()
        } else {
            "plain".to_string()
        };

        // Create a simple MIMEText instance (EmailMessage-like)
        let mut type_dict = HashMap::new();
        type_dict.insert_str(
            "as_string",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "as_string".to_string(),
                func: |a| {
                    let inst = a[0].borrow();
                    if let PyObject::Instance { dict, .. } = &*inst {
                        let content = dict
                            .get_str("_content")
                            .map(|v| v.str())
                            .unwrap_or_default();
                        let ct = dict
                            .get_str("_content_type")
                            .map(|v| v.str())
                            .unwrap_or_default();
                        let mut result = format!("Content-Type: {}\r\n", ct);
                        result.push_str(&format!("Content-Transfer-Encoding: 7bit\r\n"));
                        result.push_str("\r\n");
                        result.push_str(&content);
                        Ok(py_str(&result))
                    } else {
                        Err(PyError::type_error("MIMEText instance required"))
                    }
                },
            }),
        );

        let mime_type = PyObjectRef::new(PyObject::Type {
            name: "MIMEText".to_string(),
            dict: Box::new(str_map_to_typedict(type_dict)),
            bases: vec![],
            mro: vec![],
        });

        let mut instance_dict = AttrMap::new();
        instance_dict.insert_str("_content", py_str(&body));
        instance_dict.insert_str("_content_type", py_str(&format!("text/{}", subtype)));

        Ok(PyObjectRef::new(PyObject::Instance {
            typ: mime_type,
            dict: instance_dict,
        }))
    });

    d
}

// Zeller's congruence, adjusted for a Monday=0..Sunday=6 result (RFC 2822 order)
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

