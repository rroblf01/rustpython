use crate::object::*;
use std::collections::HashMap;

pub fn create_shutil_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! shutil_func {
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
    shutil_func!("get_terminal_size", |args| {
        // Real `shutil.get_terminal_size(fallback=(80, 24))`: prefers the
        // `COLUMNS`/`LINES` env vars, then an actual terminal query, then
        // `fallback`. This interpreter has no terminal ioctl support, so it
        // stops at the env-var check before `fallback` — good enough for
        // the overwhelmingly common real-world caller (`argparse`'s
        // `HelpFormatter`, wanting just `.columns` to wrap help text).
        let (fallback_cols, fallback_lines) = if let Some(fb) = args.get(0) {
            if let PyObject::Tuple(t) = &*fb.borrow() {
                let c = t.get(0).and_then(|v| v.as_i64()).unwrap_or(80);
                let l = t.get(1).and_then(|v| v.as_i64()).unwrap_or(24);
                (c, l)
            } else {
                (80, 24)
            }
        } else {
            (80, 24)
        };
        let columns = std::env::var("COLUMNS")
            .ok()
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(fallback_cols);
        let lines = std::env::var("LINES")
            .ok()
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(fallback_lines);
        let typ = PyObjectRef::new(PyObject::Type {
            name: "os.terminal_size".to_string(),
            dict: Box::new(str_map_to_typedict(HashMap::new())),
            bases: vec![],
            mro: vec![],
        });
        let mut dict = AttrMap::new();
        dict.insert_str("columns", py_int(columns));
        dict.insert_str("lines", py_int(lines));
        Ok(PyObjectRef::new(PyObject::Instance { typ, dict }))
    });

    shutil_func!("copy", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error(
                "copy() requires 2 arguments (src, dst)",
            ));
        }
        let src = args[0].str();
        let dst = args[1].str();
        match std::fs::copy(&src, &dst) {
            Ok(_) => Ok(py_str(&dst)),
            Err(e) => Err(PyError::os_error_from_io(&e)),
        }
    });

    // `shutil.copy2` — was missing entirely (`AttributeError`); real
    // CPython's is `copy()` plus preserved metadata (mtime/atime, and on
    // POSIX, permission bits) — `std::fs::copy` already preserves
    // permission bits on Unix, which is close enough for callers (like
    // `test_unicode_file.py`) that only check the copy exists/round-trips,
    // not that its timestamps exactly match the original.
    shutil_func!("copy2", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error(
                "copy2() requires 2 arguments (src, dst)",
            ));
        }
        let src = args[0].str();
        let dst = args[1].str();
        match std::fs::copy(&src, &dst) {
            Ok(_) => {
                // Preserve metadata like CPython: permissions + atime/mtime
                if let Ok(meta) = std::fs::metadata(&src) {
                    let _ = std::fs::set_permissions(&dst, meta.permissions());
                    let atime = filetime::FileTime::from_last_access_time(&meta);
                    let mtime = filetime::FileTime::from_last_modification_time(&meta);
                    let _ = filetime::set_file_times(&dst, atime, mtime);
                }
                Ok(py_str(&dst))
            }
            Err(e) => Err(PyError::os_error_from_io(&e)),
        }
    });

    shutil_func!("rmtree", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("rmtree() requires 1 argument (path)"));
        }
        let path = args[0].str();
        // Parse ignore_errors from positional or kwargs dict.
        let mut ignore_errors = false;
        if args.len() >= 2 && !matches!(&*args[1].borrow(), PyObject::Dict(_)) {
            ignore_errors = args[1].truthy();
        }
        for arg in args.iter() {
            if let PyObject::Dict(d) = &*arg.borrow() {
                if let Ok(Some(v)) = d.get(&py_str("ignore_errors")) {
                    ignore_errors = v.truthy();
                }
            }
        }
        match std::fs::remove_dir_all(&path) {
            Ok(()) => Ok(py_none()),
            Err(e) => {
                if ignore_errors {
                    Ok(py_none())
                } else {
                    // FileNotFoundError with ignore_errors=False should still raise,
                    // but callers that pass True expect swallow.
                    Err(PyError::os_error_from_io(&e))
                }
            }
        }
    });

    shutil_func!("move", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error(
                "move() requires 2 arguments (src, dst)",
            ));
        }
        let src = args[0].str();
        let dst = args[1].str();
        match std::fs::rename(&src, &dst) {
            Ok(()) => Ok(py_str(&dst)),
            Err(e) => Err(PyError::os_error_from_io(&e)),
        }
    });

    shutil_func!("copymode", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error(
                "copymode() requires 2 arguments (src, dst)",
            ));
        }
        let src = args[0].str();
        let dst = args[1].str();
        let perms = std::fs::metadata(&src)
            .map_err(|e| PyError::os_error_from_io(&e))?
            .permissions();
        std::fs::set_permissions(&dst, perms).map_err(|e| PyError::os_error_from_io(&e))?;
        Ok(py_none())
    });

    shutil_func!("copystat", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error(
                "copystat() requires 2 arguments (src, dst)",
            ));
        }
        let src = args[0].str();
        let dst = args[1].str();
        let meta = std::fs::metadata(&src).map_err(|e| PyError::os_error_from_io(&e))?;
        std::fs::set_permissions(&dst, meta.permissions())
            .map_err(|e| PyError::os_error_from_io(&e))?;
        let atime = filetime::FileTime::from_last_access_time(&meta);
        let mtime = filetime::FileTime::from_last_modification_time(&meta);
        filetime::set_file_times(&dst, atime, mtime)
            .map_err(|e| PyError::os_error_from_io(&e))?;
        Ok(py_none())
    });
    d
}
