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

    d
}
