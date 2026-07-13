use crate::object::*;
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH, Duration};

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

    // gmtime(secs=None) -> struct_time (simplified as tuple)
    time_func!("gmtime", |args| {
        let _secs = if !args.is_empty() { args[0].as_i64().unwrap_or(0) } else {
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64
        };
        // Return a simplified struct_time as 9-tuple
        Ok(py_tuple(vec![
            py_int(2025), py_int(1), py_int(1),  // tm_year, tm_mon, tm_mday
            py_int(0), py_int(0), py_int(0),      // tm_hour, tm_min, tm_sec
            py_int(0), py_int(0), py_int(-1),     // tm_wday, tm_yday, tm_isdst
        ]))
    });

    // localtime(secs=None) -> struct_time
    time_func!("localtime", |args| {
        let _secs = if !args.is_empty() { args[0].as_i64().unwrap_or(0) } else {
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64
        };
        Ok(py_tuple(vec![
            py_int(2025), py_int(1), py_int(1),
            py_int(0), py_int(0), py_int(0),
            py_int(0), py_int(0), py_int(-1),
        ]))
    });

    // strftime(format, struct_time) -> string
    time_func!("strftime", |args| {
        let _fmt = if !args.is_empty() { args[0].str() } else { "%c".to_string() };
        Ok(py_str("strftime stub"))
    });

    // strptime(string, format) -> struct_time
    time_func!("strptime", |args| {
        Ok(py_tuple(vec![
            py_int(2025), py_int(1), py_int(1),
            py_int(0), py_int(0), py_int(0),
            py_int(0), py_int(0), py_int(-1),
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
