use crate::object::*;
use std::collections::HashMap;
use std::sync::atomic::AtomicI64;
use num_traits::{Signed, ToPrimitive};
use std::rc::Rc;

use super::helpers::{os_path_arg, stat_dev_ino};

/// Create the os.path submodule dict with path manipulation functions.
///
/// Provides: join, exists, isfile, isdir, abspath, dirname, basename,
/// splitext, split, getsize, getmtime, islink, expanduser, normpath, normcase
pub fn create_os_path_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    d.insert_str("supports_unicode_filenames", py_bool(!cfg!(windows)));
    macro_rules! path_func {
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

    d.insert_str("curdir", py_str("."));
    d.insert_str("pardir", py_str(".."));
    d.insert_str("sep", py_str(if cfg!(windows) { "\\" } else { "/" }));
    d.insert_str(
        "altsep",
        if cfg!(windows) {
            py_str("/")
        } else {
            py_none()
        },
    );
    d.insert_str("extsep", py_str("."));
    d.insert_str("pathsep", py_str(if cfg!(windows) { ";" } else { ":" }));
    d.insert_str(
        "defpath",
        py_str(if cfg!(windows) { "." } else { ":/bin:/usr/bin" }),
    );
    d.insert_str(
        "devnull",
        py_str(if cfg!(windows) { "nul" } else { "/dev/null" }),
    );

    // --- String-based path manipulation functions ---

    path_func!("join", |args| {
        let parts: Vec<String> = args.iter().map(|a| a.str()).collect();
        if parts.is_empty() {
            return Ok(py_str(""));
        }
        let result = parts.join("/");
        Ok(py_str(&result))
    });

    path_func!("dirname", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("dirname() takes at least 1 argument"));
        }
        let path = args[0].str();
        let result = match path.rfind('/') {
            Some(i) => {
                if i == 0 {
                    "/".to_string()
                } else {
                    path[..i].to_string()
                }
            }
            None => ".".to_string(),
        };
        Ok(py_str(&result))
    });

    path_func!("basename", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("basename() takes at least 1 argument"));
        }
        let path = args[0].str();
        let result = match path.rfind('/') {
            Some(i) => path[i + 1..].to_string(),
            None => path,
        };
        Ok(py_str(&result))
    });

    path_func!("split", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("split() takes at least 1 argument"));
        }
        let path = args[0].str();
        let (head, tail) = match path.rfind('/') {
            Some(i) => {
                let h = if i == 0 {
                    "/".to_string()
                } else {
                    path[..i].to_string()
                };
                let t = path[i + 1..].to_string();
                (h, t)
            }
            None => (".".to_string(), path.clone()),
        };
        Ok(py_list(vec![py_str(&head), py_str(&tail)]))
    });

    path_func!("splitext", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("splitext() takes at least 1 argument"));
        }
        let path = args[0].str();
        let dot = path.rfind('.');
        let (root, ext) = match dot {
            Some(i) if i > path.rfind('/').map_or(0, |j| j + 1) => {
                (path[..i].to_string(), path[i..].to_string())
            }
            _ => (path.clone(), "".to_string()),
        };
        Ok(py_list(vec![py_str(&root), py_str(&ext)]))
    });

    // --- Filesystem-based checks ---

    path_func!("exists", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("exists() takes at least 1 argument"));
        }
        let p = crate::object::path_arg_to_string(&args[0]);
        Ok(py_bool(std::path::Path::new(&p).exists()))
    });

    path_func!("isfile", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("isfile() takes at least 1 argument"));
        }
        let p = crate::object::path_arg_to_string(&args[0]);
        Ok(py_bool(std::path::Path::new(&p).is_file()))
    });

    path_func!("isdir", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("isdir() takes at least 1 argument"));
        }
        let p = crate::object::path_arg_to_string(&args[0]);
        Ok(py_bool(std::path::Path::new(&p).is_dir()))
    });

    path_func!("lexists", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("lexists() takes at least 1 argument"));
        }
        let p = crate::object::path_arg_to_string(&args[0]);
        Ok(py_bool(std::fs::symlink_metadata(&p).is_ok()))
    });

    // `os.path.isabs(path)` — was missing entirely; a common, basic
    // path-classification check (does this path already start from the
    // filesystem root, or is it relative to somewhere).
    path_func!("isabs", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("isabs() takes at least 1 argument"));
        }
        Ok(py_bool(
            std::path::Path::new(&crate::object::path_arg_to_string(&args[0])).is_absolute(),
        ))
    });

    // --- Path resolution and normalization ---

    path_func!("abspath", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("abspath() takes at least 1 argument"));
        }
        let path_str = args[0].str();
        let path = std::path::Path::new(&path_str);
        if path.is_absolute() {
            // Resolve . and .. components for a clean absolute path
            let mut components: Vec<&str> = Vec::new();
            for c in path_str.split('/') {
                match c {
                    "." | "" => continue,
                    ".." => {
                        components.pop();
                    }
                    c => {
                        components.push(c);
                    }
                }
            }
            let result = if components.is_empty() {
                "/".to_string()
            } else {
                format!("/{}", components.join("/"))
            };
            Ok(py_str(&result))
        } else {
            match std::env::current_dir() {
                Ok(cwd) => {
                    let abs = cwd.join(&path_str);
                    Ok(py_str(&abs.to_string_lossy().to_string()))
                }
                Err(e) => Err(PyError::os_error_from_io(&e)),
            }
        }
    });

    // `os.path.realpath(path)` — resolves symlinks (via `std::fs::
    // canonicalize`) and returns an absolute path, falling back to the
    // plain `abspath`-style resolution above if the path doesn't exist
    // (real CPython's `realpath` doesn't require the path to exist either —
    // it resolves as much as it can and leaves the rest as-is). Missing
    // entirely before this — a common, general path-normalization idiom
    // real code reaches for constantly (not just a niche function).
    path_func!("realpath", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("realpath() takes at least 1 argument"));
        }
        let path_str = crate::object::path_arg_to_string(&args[0]);
        match std::fs::canonicalize(&path_str) {
            Ok(resolved) => Ok(py_str(&resolved.to_string_lossy())),
            Err(_) => {
                // Path doesn't exist (or a component doesn't) — fall back
                // to plain absolute-path resolution without requiring
                // existence, matching real `realpath`'s graceful behavior.
                let path = std::path::Path::new(&path_str);
                if path.is_absolute() {
                    Ok(py_str(&path_str))
                } else {
                    match std::env::current_dir() {
                        Ok(cwd) => Ok(py_str(&cwd.join(&path_str).to_string_lossy())),
                        Err(e) => Err(PyError::os_error_from_io(&e)),
                    }
                }
            }
        }
    });

    // --- Filesystem metadata ---

    path_func!("getsize", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("getsize() takes at least 1 argument"));
        }
        let path = os_path_arg(&args[0])?;
        match std::fs::metadata(&path) {
            Ok(meta) => Ok(py_int(meta.len() as i64)),
            Err(e) => Err(PyError::os_error_from_io(&e)),
        }
    });

    path_func!("getmtime", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("getmtime() takes at least 1 argument"));
        }
        let path = os_path_arg(&args[0])?;
        match std::fs::metadata(&path) {
            Ok(meta) => match meta.modified() {
                Ok(time) => {
                    use std::time::SystemTime;
                    let duration = time
                        .duration_since(SystemTime::UNIX_EPOCH)
                        .unwrap_or_default();
                    Ok(py_float(duration.as_secs_f64()))
                }
                Err(e) => Err(PyError::os_error_from_io(&e)),
            },
            Err(e) => Err(PyError::os_error_from_io(&e)),
        }
    });

    path_func!("getatime", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("getatime() takes at least 1 argument"));
        }
        let path = os_path_arg(&args[0])?;
        match std::fs::metadata(&path) {
            Ok(meta) => match meta.accessed() {
                Ok(time) => {
                    use std::time::SystemTime;
                    let duration = time
                        .duration_since(SystemTime::UNIX_EPOCH)
                        .unwrap_or_default();
                    Ok(py_float(duration.as_secs_f64()))
                }
                Err(e) => Err(PyError::os_error_from_io(&e)),
            },
            Err(e) => Err(PyError::os_error_from_io(&e)),
        }
    });

    path_func!("getctime", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("getctime() takes at least 1 argument"));
        }
        let path = os_path_arg(&args[0])?;
        match std::fs::metadata(&path) {
            Ok(meta) => {
                // On Linux `created()` is the birth time (<= mtime); close
                // enough for the "ctime <= mtime" check real callers make.
                match meta.created() {
                    Ok(time) => {
                        use std::time::SystemTime;
                        let duration = time
                            .duration_since(SystemTime::UNIX_EPOCH)
                            .unwrap_or_default();
                        Ok(py_float(duration.as_secs_f64()))
                    }
                    Err(e) => Err(PyError::os_error_from_io(&e)),
                }
            }
            Err(e) => Err(PyError::os_error_from_io(&e)),
        }
    });

    path_func!("samefile", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("samefile() takes at least 2 arguments"));
        }
        let p1 = os_path_arg(&args[0])?;
        let p2 = os_path_arg(&args[1])?;
        match (std::fs::metadata(&p1), std::fs::metadata(&p2)) {
            (Ok(m1), Ok(m2)) => {
                let (i1, d1) = stat_dev_ino(&m1);
                let (i2, d2) = stat_dev_ino(&m2);
                Ok(py_bool(i1 == i2 && d1 == d2))
            }
            (Err(e), _) | (_, Err(e)) => Err(PyError::os_error_from_io(&e)),
        }
    });

    path_func!("islink", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("islink() takes at least 1 argument"));
        }
        match std::fs::symlink_metadata(&args[0].str()) {
            Ok(meta) => Ok(py_bool(meta.file_type().is_symlink())),
            Err(_) => Ok(py_bool(false)), // Python os.path.islink returns False on error
        }
    });

    // --- User expansion ---

    path_func!("expanduser", |args| {
        if args.is_empty() {
            return Err(PyError::type_error(
                "expanduser() takes at least 1 argument",
            ));
        }
        let path = args[0].str();
        if path == "~" || path.starts_with("~/") {
            match std::env::var("HOME") {
                Ok(home) => {
                    let result = if path == "~" {
                        home
                    } else {
                        format!("{}/{}", home, &path[2..])
                    };
                    Ok(py_str(&result))
                }
                Err(_) => Ok(py_str(&path)),
            }
        } else {
            Ok(py_str(&path))
        }
    });

    // --- Normalization ---

    path_func!("normpath", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("normpath() takes at least 1 argument"));
        }
        let path = args[0].str();
        let mut parts: Vec<&str> = Vec::new();
        let is_absolute = path.starts_with('/');
        for segment in path.split('/') {
            match segment {
                "." | "" => continue,
                ".." => {
                    // Only pop if we won't go above root (for absolute paths)
                    // or if we have a regular component to pop (for relative)
                    if parts.is_empty() {
                        if !is_absolute {
                            parts.push("..");
                        }
                        // else: absolute path, just ignore (can't go above /)
                    } else if parts.last() == Some(&"..") {
                        parts.push("..");
                    } else {
                        parts.pop();
                    }
                }
                c => parts.push(c),
            }
        }
        let joined = parts.join("/");
        let result = if is_absolute {
            format!("/{}", joined)
        } else if joined.is_empty() {
            ".".to_string()
        } else {
            joined
        };
        Ok(py_str(&result))
    });

    path_func!("normcase", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("normcase() takes at least 1 argument"));
        }
        let path = args[0].str();
        // On Unix, normcase is a no-op (returns path unchanged)
        // On Windows it would lowercase and convert / to \\
        Ok(py_str(&path))
    });

    // commonprefix(list) — longest literal (character-wise, not
    // path-component-aware) string prefix shared by every path in `list`.
    // Was missing entirely — needed by the real `unittest.util` module
    // (`from os.path import commonprefix`, used for diffing assertion
    // messages), which is itself needed by any real `unittest`-based test
    // suite (Django's own test framework included).
    path_func!("commonprefix", |args| {
        if args.is_empty() {
            return Err(PyError::type_error(
                "commonprefix() takes at least 1 argument",
            ));
        }
        let paths: Vec<String> = crate::object::collect_iterable(&args[0])?
            .iter()
            .map(|p| crate::object::path_arg_to_string(p))
            .collect();
        if paths.is_empty() {
            return Ok(py_str(""));
        }
        let first = &paths[0];
        let mut prefix_len = first.chars().count();
        for p in &paths[1..] {
            let common = first
                .chars()
                .zip(p.chars())
                .take_while(|(a, b)| a == b)
                .count();
            prefix_len = prefix_len.min(common);
        }
        let prefix: String = first.chars().take(prefix_len).collect();
        Ok(py_str(&prefix))
    });

    // os.path.splitdrive(p): Split a pathname into drive/UNC sharepoint
    // and relative path specifiers. On POSIX, the drive is always empty.
    path_func!("splitdrive", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("splitdrive() missing required argument: 'p'"));
        }
        let p = args[0].str();
        Ok(py_tuple(vec![py_str(""), py_str(&p)]))
    });

    // --- Additional POSIX path functions for CPython 3.11+ compat ---
    // splitroot: POSIX uses single sep semantics; mimic Lib/posixpath.splitroot
    path_func!("splitroot", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("splitroot() missing required argument: 'p'"));
        }
        let p = crate::object::path_arg_to_string(&args[0]);
        if !p.starts_with('/') {
            return Ok(py_tuple(vec![py_str(""), py_str(""), py_str(&p)]));
        } else if p.len() >= 2 && p.as_bytes()[1] != b'/' || p.len() >= 3 && p.as_bytes()[2] == b'/' {
            // Absolute path, e.g. '/foo', '///foo'
            return Ok(py_tuple(vec![py_str(""), py_str("/"), py_str(&p[1..])]));
        } else {
            // Precisely two leading slashes, e.g. '//foo' (implementation-defined)
            return Ok(py_tuple(vec![py_str(""), py_str("//"), py_str(&p[2..])]));
        }
    });

    path_func!("commonpath", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("commonpath() takes at least 1 argument"));
        }
        let paths: Vec<String> = crate::object::collect_iterable(&args[0])?
            .iter()
            .map(|p| crate::object::path_arg_to_string(p))
            .collect();
        if paths.is_empty() {
            return Err(PyError::value_error("commonpath() arg is an empty sequence"));
        }
        let isabs = paths[0].starts_with('/');
        for p in &paths {
            if p.starts_with('/') != isabs {
                return Err(PyError::value_error("Can't mix absolute and relative paths"));
            }
        }
        let split_paths: Vec<Vec<String>> = paths
            .iter()
            .map(|p| {
                p.split('/')
                    .filter(|c| !c.is_empty() && *c != ".")
                    .map(|s| s.to_string())
                    .collect()
            })
            .collect();
        let s1 = split_paths.iter().min().cloned().unwrap_or_default();
        let s2 = split_paths.iter().max().cloned().unwrap_or_default();
        let mut common = s1.clone();
        for (i, c) in s1.iter().enumerate() {
            if i >= s2.len() || c != &s2[i] {
                common = s1[..i].to_vec();
                break;
            }
        }
        let prefix = if isabs { "/" } else { "" };
        let result = if common.is_empty() {
            prefix.to_string()
        } else {
            format!("{}{}", prefix, common.join("/"))
        };
        Ok(py_str(&result))
    });

    path_func!("relpath", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("relpath() missing required argument: 'path'"));
        }
        // Handle optional second arg or kw `start`
        let mut path = crate::object::path_arg_to_string(&args[0]);
        let mut start = if args.len() >= 2 {
            // Check if second arg is a kwargs dict
            if args.len() == 2 {
                if let PyObject::Dict(d) = &*args[1].borrow() {
                    // Might be kwargs dict containing "start"
                    if let Some(v) = d.get(&py_str("start")).ok().flatten() {
                        crate::object::path_arg_to_string(&v)
                    } else if d.contains(&py_str("start")).unwrap_or(false) {
                        ".".to_string()
                    } else {
                        // Not a kwargs dict, treat as positional start? But we passed dict as start => not valid path
                        // Check if this is actually a kwargs container - if it has any key, treat as kwargs
                        if d.len() > 0 {
                            ".".to_string()
                        } else {
                            crate::object::path_arg_to_string(&args[1])
                        }
                    }
                } else {
                    crate::object::path_arg_to_string(&args[1])
                }
            } else {
                crate::object::path_arg_to_string(&args[1])
            }
        } else {
            ".".to_string()
        };
        // Also check trailing kwargs dict when 3 args (path, start, kwargs)
        if args.len() >= 3 {
            if let PyObject::Dict(d) = &*args[args.len() - 1].borrow() {
                if let Some(v) = d.get(&py_str("start")).ok().flatten() {
                    start = crate::object::path_arg_to_string(&v);
                }
            }
        }
        if path.is_empty() {
            return Err(PyError::value_error("no path specified"));
        }
        // Resolve to absolute and strip leading /
        let abspath_fn = |p: &str| -> String {
            let abs = if p.starts_with('/') {
                p.to_string()
            } else {
                match std::env::current_dir() {
                    Ok(cwd) => format!("{}/{}", cwd.to_string_lossy(), p),
                    Err(_) => p.to_string(),
                }
            };
            // normpath-like cleaning
            let mut parts: Vec<&str> = Vec::new();
            let is_abs = abs.starts_with('/');
            for seg in abs.split('/') {
                match seg {
                    "" | "." => continue,
                    ".." => { parts.pop(); }
                    c => parts.push(c),
                }
            }
            if is_abs {
                format!("/{}", parts.join("/"))
            } else if parts.is_empty() {
                ".".to_string()
            } else {
                parts.join("/")
            }
        };
        let start_abs = abspath_fn(&start);
        let path_abs = abspath_fn(&path);
        let start_tail = start_abs.trim_start_matches('/').to_string();
        let path_tail = path_abs.trim_start_matches('/').to_string();
        let start_list: Vec<&str> = if start_tail.is_empty() { vec![] } else { start_tail.split('/').collect() };
        let path_list: Vec<&str> = if path_tail.is_empty() { vec![] } else { path_tail.split('/').collect() };
        // commonprefix
        let mut i = 0;
        for (a, b) in start_list.iter().zip(path_list.iter()) {
            if a != b { break; }
            i += 1;
        }
        let mut rel: Vec<String> = std::iter::repeat("..".to_string()).take(start_list.len() - i).collect();
        rel.extend(path_list[i..].iter().map(|s| s.to_string()));
        if rel.is_empty() {
            Ok(py_str("."))
        } else {
            Ok(py_str(&rel.join("/")))
        }
    });

    path_func!("expandvars", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("expandvars() missing required argument: 'path'"));
        }
        let path = crate::object::path_arg_to_string(&args[0]);
        // Simple POSIX expansion: $var and ${var}
        // Find $ patterns
        let mut result = String::new();
        let mut chars = path.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '$' {
                if let Some(&next) = chars.peek() {
                    if next == '{' {
                        chars.next(); // consume {
                        let mut var = String::new();
                        let mut closed = false;
                        for ch in chars.by_ref() {
                            if ch == '}' { closed = true; break; }
                            var.push(ch);
                        }
                        if closed {
                            if let Ok(val) = std::env::var(&var) {
                                result.push_str(&val);
                            } else {
                                result.push_str(&format!("${{{}}}", var));
                            }
                        } else {
                            result.push_str(&format!("${{{}", var));
                        }
                    } else if next.is_alphanumeric() || next == '_' {
                        let mut var = String::new();
                        while let Some(&ch) = chars.peek() {
                            if ch.is_alphanumeric() || ch == '_' {
                                var.push(ch);
                                chars.next();
                            } else { break; }
                        }
                        if let Ok(val) = std::env::var(&var) {
                            result.push_str(&val);
                        } else {
                            result.push_str(&format!("${}", var));
                        }
                    } else {
                        result.push(c);
                    }
                } else {
                    result.push(c);
                }
            } else {
                result.push(c);
            }
        }
        Ok(py_str(&result))
    });

    path_func!("ismount", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("ismount() missing required argument: 'path'"));
        }
        let p = crate::object::path_arg_to_string(&args[0]);
        // Use stat-based heuristic: compare st_dev of path and parent
        let path_stat = std::fs::metadata(&p).ok();
        let parent = std::path::Path::new(&p).parent().map(|pp| pp.to_string_lossy().to_string()).unwrap_or("/".to_string());
        let parent_stat = std::fs::metadata(&parent).ok();
        if let (Some(ps), Some(pps)) = (path_stat, parent_stat) {
            let (ino1, dev1) = stat_dev_ino(&ps);
            let (ino2, dev2) = stat_dev_ino(&pps);
            // Different device or same inode indicates mount
            Ok(py_bool(dev1 != dev2 || ino1 == ino2))
        } else {
            Ok(py_bool(false))
        }
    });

    path_func!("isjunction", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("isjunction() missing required argument: 'path'"));
        }
        // Junctions only on Windows; always false on POSIX
        let _ = crate::object::path_arg_to_string(&args[0]);
        Ok(py_bool(false))
    });

    path_func!("isdevdrive", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("isdevdrive() missing required argument: 'path'"));
        }
        let _ = crate::object::path_arg_to_string(&args[0]);
        Ok(py_bool(false))
    });

    path_func!("samestat", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("samestat() takes at least 2 arguments"));
        }
        // args are stat result objects; compare st_ino and st_dev
        let s1 = &args[0];
        let s2 = &args[1];
        let get = |obj: &PyObjectRef, key: &str| -> Option<u64> {
            if let Ok(val) = obj.borrow().get_attribute(key) {
                val.as_i64().map(|v| v as u64).or_else(|| {
                    if let PyObject::Float(f) = &*val.borrow() { Some(*f as u64) } else { None }
                })
            } else { None }
        };
        // Fallback to dict access if PyObject::Dict
        let get_dict = |obj: &PyObjectRef, key: &str| -> Option<u64> {
            if let PyObject::Dict(d) = &*obj.borrow() {
                if let Ok(Some(v)) = d.get(&py_str(key)) {
                    return v.as_i64().map(|x| x as u64);
                }
            }
            get(obj, key)
        };
        let ino1 = get_dict(s1, "st_ino").or_else(|| get(s1, "st_ino"));
        let ino2 = get_dict(s2, "st_ino").or_else(|| get(s2, "st_ino"));
        let dev1 = get_dict(s1, "st_dev").or_else(|| get(s1, "st_dev"));
        let dev2 = get_dict(s2, "st_dev").or_else(|| get(s2, "st_dev"));
        match (ino1, ino2, dev1, dev2) {
            (Some(i1), Some(i2), Some(d1), Some(d2)) => Ok(py_bool(i1 == i2 && d1 == d2)),
            _ => Ok(py_bool(false)),
        }
    });

    path_func!("sameopenfile", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("sameopenfile() takes at least 2 arguments"));
        }
        // args are file objects or fds; compare via fstat dev/ino if possible
        let get_fd = |obj: &PyObjectRef| -> Option<i32> {
            // Try fileno() method
            if let Ok(m) = obj.borrow().get_attribute("fileno") {
                if let Ok(res) = crate::object::call_bound_method(m.clone(), obj.clone(), vec![]) {
                    return res.as_i64().map(|v| v as i32);
                }
            }
            obj.as_i64().map(|v| v as i32)
        };
        if let (Some(fd1), Some(fd2)) = (get_fd(&args[0]), get_fd(&args[1])) {
            let p1 = format!("/proc/self/fd/{}", fd1);
            let p2 = format!("/proc/self/fd/{}", fd2);
            if let (Ok(m1), Ok(m2)) = (std::fs::metadata(&p1), std::fs::metadata(&p2)) {
                let (i1, d1) = stat_dev_ino(&m1);
                let (i2, d2) = stat_dev_ino(&m2);
                return Ok(py_bool(i1 == i2 && d1 == d2));
            }
        }
        // Fallback: compare objects identity?
        Ok(py_bool(false))
    });

    // Overwrite realpath to handle `strict` keyword (including ALLOW_MISSING sentinel)
    // The earlier realpath defined above is replaced here with a strict-aware version.
    {
        let key = "realpath".to_string();
        d.insert(
            key.clone(),
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: key,
                func: |args| {
                    if args.is_empty() {
                        return Err(PyError::type_error("realpath() takes at least 1 argument"));
                    }
                    let path_str = crate::object::path_arg_to_string(&args[0]);
                    // Parse strict param from trailing kwargs dict or positional
                    let mut strict_val: Option<PyObjectRef> = None;
                    if args.len() >= 2 {
                        // Check if last arg is a kwargs dict containing 'strict'
                        if let PyObject::Dict(d) = &*args[args.len() - 1].borrow() {
                            if let Ok(Some(v)) = d.get(&py_str("strict")) {
                                strict_val = Some(v.clone());
                            } else if args.len() == 2 {
                                // Single extra arg could be either strict value or empty kwargs
                                // Heuristic: if dict has no 'strict' key but is kwargs container, ignore
                                // If dict is empty, it was an empty kwargs dict, no strict
                                if d.len() == 0 {
                                    strict_val = None;
                                } else {
                                    strict_val = None;
                                }
                            }
                            // Also handle positional strict when args has 3 elements: path, strict_val, kwargs_dict
                            if args.len() == 3 {
                                // args[1] is positional strict, args[2] is kwargs
                                strict_val = Some(args[1].clone());
                                if let PyObject::Dict(d2) = &*args[2].borrow() {
                                    if let Ok(Some(v)) = d2.get(&py_str("strict")) {
                                        strict_val = Some(v.clone());
                                    }
                                }
                            }
                        } else {
                            // Positional strict
                            strict_val = Some(args[1].clone());
                        }
                    }
                    // Determine strict mode
                    let is_allow_missing = if let Some(ref sv) = strict_val {
                        // Check identity via string repr "os.path.ALLOW_MISSING" for cross-module sentinel
                        let s = sv.str();
                        s == "os.path.ALLOW_MISSING" || s == "ALLOW_MISSING" ||
                        // Also check if the object is the native sentinel string
                        sv.borrow().get_attribute("__repr__").ok().map(|r| {
                            if let Ok(rr) = crate::object::call_bound_method(r.clone(), sv.clone(), vec![]) { rr.str() == "os.path.ALLOW_MISSING" } else { false }
                        }).unwrap_or(false) ||
                        // Direct ptr equality with native sentinel string will be handled via s check above
                        false
                    } else { false };
                    let is_strict_true = if let Some(ref sv) = strict_val {
                        if is_allow_missing { false } else {
                            // truthy check
                            sv.try_truthy().unwrap_or(false)
                        }
                    } else { false };

                    // If strict is True (not ALLOW_MISSING), we should raise if path doesn't exist? But for now, just canonicalize regardless.
                    // Handle ALLOW_MISSING similar to CPython's posixpath: ignore FileNotFoundError, otherwise strict error.
                    match std::fs::canonicalize(&path_str) {
                        Ok(resolved) => Ok(py_str(&resolved.to_string_lossy())),
                        Err(e) => {
                            if is_strict_true {
                                return Err(PyError::os_error_from_io(&e));
                            }
                            // Non-strict or ALLOW_MISSING: fallback to abspath-like resolution
                            let path = std::path::Path::new(&path_str);
                            if path.is_absolute() {
                                Ok(py_str(&path_str))
                            } else {
                                match std::env::current_dir() {
                                    Ok(cwd) => Ok(py_str(&cwd.join(&path_str).to_string_lossy())),
                                    Err(e) => Err(PyError::os_error_from_io(&e)),
                                }
                            }
                        }
                    }
                },
            }),
        );
    }

    // ALLOW_MISSING sentinel — must be after realpath overwrite so realpath can check it
    // Use a string sentinel "os.path.ALLOW_MISSING"; realpath checks str equality, covering both native and genericpath sentinels.
    d.insert_str("ALLOW_MISSING", py_str("os.path.ALLOW_MISSING"));

    d
}

