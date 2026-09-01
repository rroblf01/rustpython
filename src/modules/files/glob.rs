use crate::object::*;
use std::collections::HashMap;

pub fn create_glob_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! glob_func {
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

    fn glob_match(name: &str, pattern: &str) -> bool {
        let re_str = format!(
            "^{}$",
            pattern
                .replace(".", "\\.")
                .replace("?", ".")
                .replace("*", ".*")
        );
        regex::Regex::new(&re_str)
            .map(|re| re.is_match(name))
            .unwrap_or(false)
    }

    fn walk_glob(base: &std::path::Path, parts: &[&str], prefix: &str, results: &mut Vec<String>) {
        if parts.is_empty() {
            return;
        }
        let part = parts[0];
        let rest = &parts[1..];

        if let Ok(entries) = std::fs::read_dir(base) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if !glob_match(&name, part) {
                    continue;
                }
                let full = if prefix.is_empty() {
                    name.clone()
                } else {
                    format!("{}/{}", prefix, name)
                };
                if rest.is_empty() {
                    results.push(full);
                } else {
                    let path = entry.path();
                    if path.is_dir() {
                        walk_glob(&path, rest, &full, results);
                    }
                }
            }
        }
    }

    glob_func!("glob", |args| {
        if args.len() < 1 {
            return Err(PyError::type_error("glob() takes exactly 1 argument"));
        }
        let pattern = args[0].str();
        let pattern = pattern.trim().to_string();
        if pattern.is_empty() {
            return Ok(py_list(vec![]));
        }

        let is_absolute = pattern.starts_with('/');
        let parts: Vec<&str> = pattern.split('/').filter(|p| !p.is_empty()).collect();
        if parts.is_empty() {
            return Ok(py_list(vec![]));
        }

        let start = if is_absolute {
            std::path::Path::new("/")
        } else {
            std::path::Path::new(".")
        };

        let mut results = Vec::new();
        if let Ok(entries) = std::fs::read_dir(start) {
            let first = parts[0];
            let rest = &parts[1..];
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if !glob_match(&name, first) {
                    continue;
                }
                if rest.is_empty() {
                    results.push(name);
                } else {
                    let path = entry.path();
                    if path.is_dir() {
                        walk_glob(&path, rest, &name, &mut results);
                    }
                }
            }
        }

        results.sort();
        let py_results: Vec<PyObjectRef> = results.into_iter().map(|s| py_str(&s)).collect();
        Ok(py_list(py_results))
    });

    // `glob.iglob(pattern)` — generator form of glob; delegates to the same
    // matching, returns a list iterator.
    glob_func!("iglob", |args| {
        if args.len() < 1 {
            return Err(PyError::type_error("iglob() takes exactly 1 argument"));
        }
        let pattern = args[0].str();
        let pattern = pattern.trim().to_string();
        if pattern.is_empty() {
            return Ok(PyObjectRef::new(PyObject::ListIter {
                list: vec![],
                index: 0,
            }));
        }
        let is_absolute = pattern.starts_with('/');
        let parts: Vec<&str> = pattern.split('/').filter(|p| !p.is_empty()).collect();
        if parts.is_empty() {
            return Ok(PyObjectRef::new(PyObject::ListIter {
                list: vec![],
                index: 0,
            }));
        }
        let start = if is_absolute {
            std::path::Path::new("/")
        } else {
            std::path::Path::new(".")
        };
        let mut results = Vec::new();
        if let Ok(entries) = std::fs::read_dir(start) {
            let first = parts[0];
            let rest = &parts[1..];
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if !glob_match(&name, first) {
                    continue;
                }
                if rest.is_empty() {
                    results.push(name);
                } else {
                    let path = entry.path();
                    if path.is_dir() {
                        walk_glob(&path, rest, &name, &mut results);
                    }
                }
            }
        }
        results.sort();
        let py_results: Vec<PyObjectRef> = results.into_iter().map(|s| py_str(&s)).collect();
        Ok(PyObjectRef::new(PyObject::ListIter {
            list: py_results,
            index: 0,
        }))
    });

    // `glob.glob0(dirname, basename)` / `glob.glob1(dirname, pattern)` — the
    // deprecated 3.14-era private helpers test_glob.py exercises directly.
    // glob0: returns [basename] if the literal joined path exists, else [].
    // glob1: lists dirname and fnmatch-filters the names (skipping hidden
    // entries unless the pattern itself starts with a dot). Both issue a
    // DeprecationWarning like CPython.
    glob_func!("glob0", |args| {
        let warn = warn_deprecated_glob();
        let dirname = args[0].str();
        let basename = args[1].str();
        if basename.is_empty() {
            if std::path::Path::new(&dirname).is_dir() {
                return Ok(py_list(vec![py_str(&dirname.to_string())]));
            }
            return Ok(py_list(vec![]));
        }
        let full = std::path::Path::new(&dirname).join(&basename);
        if full.exists() {
            Ok(py_list(vec![py_str(&basename.to_string())]))
        } else {
            Ok(py_list(vec![]))
        }
    });
    glob_func!("glob1", |args| {
        let warn = warn_deprecated_glob();
        let dirname = args[0].str();
        let pattern = args[1].str();
        let mut names: Vec<String> = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&dirname) {
            for entry in entries.flatten() {
                names.push(entry.file_name().to_string_lossy().to_string());
            }
        }
        names.sort();
        let include_hidden = pattern.starts_with('.');
        let re_str = format!(
            "^{}$",
            pattern
                .replace(".", "\\.")
                .replace("?", ".")
                .replace("*", ".*")
        );
        let re = regex::Regex::new(&re_str).ok();
        let mut results: Vec<PyObjectRef> = Vec::new();
        for name in names {
            if !include_hidden && name.starts_with('.') {
                continue;
            }
            if let Some(re) = &re {
                if re.is_match(&name) {
                    results.push(py_str(&name));
                }
            }
        }
        Ok(py_list(results))
    });

    // `glob.escape(pathname)` — was missing entirely (`AttributeError`).
    // Real semantics: wrap each special glob character (`*`, `?`, `[`) in
    // its own single-char bracket class (`[*]`) so it's matched literally
    // rather than interpreted as a glob wildcard — used by
    // `test_unicode_file.py` to safely glob a path that might itself
    // contain such characters.
    glob_func!("escape", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("escape() takes exactly 1 argument"));
        }
        let is_bytes = matches!(&*args[0].borrow(), PyObject::Bytes(_));
        let raw: String = if is_bytes {
            // For bytes args use the raw bytes content, not str(b) == b'abc'.
            let b = args[0].borrow();
            if let PyObject::Bytes(v) = &*b {
                String::from_utf8_lossy(v).to_string()
            } else {
                String::new()
            }
        } else {
            args[0].str()
        };
        let mut out = String::new();
        for c in raw.chars() {
            if c == '*' || c == '?' || c == '[' {
                out.push('[');
                out.push(c);
                out.push(']');
            } else {
                out.push(c);
            }
        }
        if is_bytes {
            Ok(PyObjectRef::imm(PyObject::Bytes(out.into_bytes())))
        } else {
            Ok(py_str(&out))
        }
    });

    fn warn_deprecated_glob() {
        // Emit DeprecationWarning("glob.glob0 is deprecated...") via the
        // warnings machinery; swallow failures (module missing).
        use std::cell::RefCell;
        thread_local! {
            static WARNED: RefCell<bool> = RefCell::new(false);
        }
        WARNED.with(|w| {
            let mut w = w.borrow_mut();
            if *w {
                return;
            }
            *w = true;
            if let Some(module) = crate::modules::get_module("warnings") {
                let _ = module.borrow().get_attribute("warn").and_then(|warn| {
                    crate::object::call_function_disposable(
                        &warn,
                        vec![
                            py_str("glob.glob0 is deprecated; use glob.glob with root_dir instead"),
                            crate::modules::get_builtin_class("DeprecationWarning")
                                .unwrap_or(py_none()),
                        ],
                        vec![],
                    )
                });
            }
        });
    }

    d
}
