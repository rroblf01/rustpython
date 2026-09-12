use crate::object::*;
use num_traits::ToPrimitive;
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::rc::Rc;

// Moved here from object.rs (was under a "---- pathlib module ----" banner in
// the monolithic object.rs, alongside other misplaced stdlib-module code —
// see the file-splitting refactor's memory entry for context).
thread_local! {
    pub static PATH_TYPE: std::cell::RefCell<Option<PyObjectRef>> = std::cell::RefCell::new(None);
}

mod glob;
pub use glob::*;
mod fnmatch;
pub use fnmatch::*;
mod shutil;
pub use shutil::*;
mod gzip;
pub use gzip::*;
mod tarfile;
pub use tarfile::*;
mod zipfile;
pub use zipfile::*;
mod shelve;
pub use shelve::*;
mod linecache;
pub use linecache::*;

pub fn create_pathlib_dict() -> HashMap<String, PyObjectRef> {
    let mut path_type_dict = HashMap::new();

    macro_rules! path_func {
        ($name:expr, $func:expr) => {
            path_type_dict.insert(
                $name.to_string(),
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: $name.to_string(),
                    func: $func,
                }),
            );
        };
    }

    // Helper to get the path string from a Path instance
    fn path_instance_str(instance: &PyObjectRef) -> String {
        instance
            .borrow()
            .get_attribute("_path")
            .map(|v| v.str())
            .unwrap_or_default()
    }

    // __str__: str(path) returns the path string
    path_func!("__str__", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("__str__() missing argument"));
        }
        Ok(py_str(&path_instance_str(&args[0])))
    });

    // __repr__: repr(path)
    path_func!("__repr__", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("__repr__() missing argument"));
        }
        let s = path_instance_str(&args[0]);
        Ok(py_str(&format!("PurePosixPath('{}')", s)))
    });

    // __init__: Path(path_str) stores the path string. Real pathlib
    // accepts any `os.PathLike` (an object with `__fspath__`), not just a
    // plain `str`/`Path` — `.str()` on such an object (e.g. `test_zipapp`'s
    // `os_helper.FakePath`) would stringify its `repr()`/generic object
    // string, not the actual path, so `__fspath__` must be tried first.
    path_func!("__init__", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("__init__() missing path argument"));
        }
        let path_val = if args.len() > 1 {
            let arg = &args[1];
            let s = match &*arg.borrow() {
                PyObject::Str(s) => s.to_string(),
                _ => {
                    // `get_attribute` (unlike bytecode `LOAD_ATTR`) returns
                    // the raw, UNBOUND method — self must be passed
                    // explicitly regardless of whether the target is our
                    // own native `Path.__fspath__` (`args[0]`-as-self
                    // convention) or a genuine Python-defined `__fspath__`
                    // (a real `def __fspath__(self)`).
                    if let Ok(fspath) = arg.borrow().get_attribute("__fspath__") {
                        crate::object::builtin_call(&fspath, &[arg.clone()])?.str()
                    } else {
                        arg.str()
                    }
                }
            };
            py_str(&s)
        } else {
            py_str(".")
        };
        if let PyObject::Instance { dict, .. } = &mut *args[0].borrow_mut() {
            dict.insert_str("_path", path_val);
        }
        Ok(py_none())
    });

    // .parent -> dirname (property getter). Real pathlib's `.parent`
    // returns another `Path` object (not a plain `str`) — real code
    // routinely chains straight off it (`Path(__file__).parent / 'x'`, the
    // single most common pathlib idiom, confirmed via CPython's own
    // `test_traceback.py`'s module-level `LEVENSHTEIN_DATA_FILE = Path(
    // __file__).parent / 'levenshtein_examples.json'`) — returning a bare
    // string here meant every such chain hit `/`'s `'str' and 'str'`
    // TypeError right after the `Path / str` fix above stopped masking it.
    {
        let getter = PyObjectRef::new(PyObject::BuiltinFunction {
            name: "parent".to_string(),
            func: |args| {
                if args.is_empty() {
                    return Err(PyError::type_error("parent getter missing argument"));
                }
                let s = path_instance_str(&args[0]);
                let parent = std::path::Path::new(&s)
                    .parent()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default();
                let path_type = PATH_TYPE
                    .with(|cell| cell.borrow().clone())
                    .ok_or_else(|| {
                        PyError::runtime_error("Path type not initialized".to_string())
                    })?;
                let mut instance_dict = AttrMap::new();
                instance_dict.insert_str("_path", py_str(&parent));
                Ok(PyObjectRef::new(PyObject::Instance {
                    typ: path_type,
                    dict: instance_dict,
                }))
            },
        });
        path_type_dict.insert_str(
            "parent",
            PyObjectRef::new(PyObject::Property(Box::new(PropertyData {
                getter: Some(getter),
                setter: None,
                deleter: None,
                doc: None,
            }))),
        );
    }

    // .name -> basename (file or last component, property getter)
    {
        let getter = PyObjectRef::new(PyObject::BuiltinFunction {
            name: "name".to_string(),
            func: |args| {
                if args.is_empty() {
                    return Err(PyError::type_error("name getter missing argument"));
                }
                let s = path_instance_str(&args[0]);
                let name = std::path::Path::new(&s)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                Ok(py_str(&name))
            },
        });
        path_type_dict.insert_str(
            "name",
            PyObjectRef::new(PyObject::Property(Box::new(PropertyData {
                getter: Some(getter),
                setter: None,
                deleter: None,
                doc: None,
            }))),
        );
    }

    // .suffix -> extension (e.g. ".txt", property getter)
    {
        let getter = PyObjectRef::new(PyObject::BuiltinFunction {
            name: "suffix".to_string(),
            func: |args| {
                if args.is_empty() {
                    return Err(PyError::type_error("suffix getter missing argument"));
                }
                let s = path_instance_str(&args[0]);
                let suffix = std::path::Path::new(&s)
                    .extension()
                    .map(|e| format!(".{}", e.to_string_lossy()))
                    .unwrap_or_default();
                Ok(py_str(&suffix))
            },
        });
        path_type_dict.insert_str(
            "suffix",
            PyObjectRef::new(PyObject::Property(Box::new(PropertyData {
                getter: Some(getter),
                setter: None,
                deleter: None,
                doc: None,
            }))),
        );
    }

    // .stem -> filename without extension (property getter)
    {
        let getter = PyObjectRef::new(PyObject::BuiltinFunction {
            name: "stem".to_string(),
            func: |args| {
                if args.is_empty() {
                    return Err(PyError::type_error("stem getter missing argument"));
                }
                let s = path_instance_str(&args[0]);
                let stem = std::path::Path::new(&s)
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default();
                Ok(py_str(&stem))
            },
        });
        path_type_dict.insert_str(
            "stem",
            PyObjectRef::new(PyObject::Property(Box::new(PropertyData {
                getter: Some(getter),
                setter: None,
                deleter: None,
                doc: None,
            }))),
        );
    }

    // .exists() -> bool
    path_func!("exists", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("exists() missing argument"));
        }
        let s = path_instance_str(&args[0]);
        Ok(py_bool(std::path::Path::new(&s).exists()))
    });

    // .is_file() -> bool
    path_func!("is_file", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("is_file() missing argument"));
        }
        let s = path_instance_str(&args[0]);
        Ok(py_bool(std::path::Path::new(&s).is_file()))
    });

    // .is_dir() -> bool
    path_func!("is_dir", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("is_dir() missing argument"));
        }
        let s = path_instance_str(&args[0]);
        Ok(py_bool(std::path::Path::new(&s).is_dir()))
    });

    // .joinpath(*parts) -> new Path
    path_func!("joinpath", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("joinpath() missing argument"));
        }
        let mut base = std::path::PathBuf::from(path_instance_str(&args[0]));
        for part in args.iter().skip(1) {
            base.push(part.str());
        }
        let result = base.to_string_lossy().to_string();
        // Get Path type from thread_local and create a new Path instance
        let path_type = PATH_TYPE
            .with(|cell| cell.borrow().clone())
            .ok_or_else(|| PyError::runtime_error("Path type not initialized".to_string()))?;
        let mut instance_dict = AttrMap::new();
        instance_dict.insert_str("_path", py_str(&result));
        Ok(PyObjectRef::new(PyObject::Instance {
            typ: path_type,
            dict: instance_dict,
        }))
    });

    // `Path(...) / 'segment'` — the single most common pathlib idiom in
    // real code — was missing entirely (`__truediv__` not defined at all),
    // so any real path-joining-via-`/` code raised `TypeError: unsupported
    // operand type(s) for /: 'instance' and 'str'`. Same join logic as
    // `joinpath` just above (duplicated rather than shared — this
    // codebase's `path_func!` macro wraps each closure directly into a
    // `BuiltinFunction` fn pointer, so closures here can't call each other).
    path_func!("__truediv__", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("__truediv__() missing argument"));
        }
        let mut base = std::path::PathBuf::from(path_instance_str(&args[0]));
        base.push(args[1].str());
        let result = base.to_string_lossy().to_string();
        let path_type = PATH_TYPE
            .with(|cell| cell.borrow().clone())
            .ok_or_else(|| PyError::runtime_error("Path type not initialized".to_string()))?;
        let mut instance_dict = AttrMap::new();
        instance_dict.insert_str("_path", py_str(&result));
        Ok(PyObjectRef::new(PyObject::Instance {
            typ: path_type,
            dict: instance_dict,
        }))
    });

    // `'segment' / Path(...)` — the reflected form (real pathlib supports
    // this via `Path.__rtruediv__`, prepending the left-hand string).
    path_func!("__rtruediv__", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("__rtruediv__() missing argument"));
        }
        let mut base = std::path::PathBuf::from(args[1].str());
        base.push(path_instance_str(&args[0]));
        let result = base.to_string_lossy().to_string();
        let path_type = PATH_TYPE
            .with(|cell| cell.borrow().clone())
            .ok_or_else(|| PyError::runtime_error("Path type not initialized".to_string()))?;
        let mut instance_dict = AttrMap::new();
        instance_dict.insert_str("_path", py_str(&result));
        Ok(PyObjectRef::new(PyObject::Instance {
            typ: path_type,
            dict: instance_dict,
        }))
    });

    // .resolve(strict=False) -> new Path with resolved absolute path (resolves symlinks)
    path_func!("resolve", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("resolve() missing argument"));
        }
        let s = path_instance_str(&args[0]);
        let _strict = if args.len() > 1 {
            match &*args[1].borrow() {
                PyObject::Bool(b) => *b,
                _ => false,
            }
        } else {
            false
        };
        let result = match std::path::Path::new(&s).canonicalize() {
            Ok(p) => p.to_string_lossy().to_string(),
            Err(_) => {
                // Fallback: current_dir + path (mirrors absolute() behavior)
                let cwd = std::env::current_dir()
                    .map(|d| d.to_string_lossy().to_string())
                    .unwrap_or_default();
                let joined = std::path::Path::new(&cwd).join(&s);
                joined.to_string_lossy().to_string()
            }
        };
        let path_type = PATH_TYPE
            .with(|cell| cell.borrow().clone())
            .ok_or_else(|| PyError::runtime_error("Path type not initialized".to_string()))?;
        let mut instance_dict = AttrMap::new();
        instance_dict.insert_str("_path", py_str(&result));
        Ok(PyObjectRef::new(PyObject::Instance {
            typ: path_type,
            dict: instance_dict,
        }))
    });

    // .absolute() -> new Path with absolute path
    path_func!("absolute", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("absolute() missing argument"));
        }
        let s = path_instance_str(&args[0]);
        let result = match std::path::Path::new(&s).canonicalize() {
            Ok(p) => p.to_string_lossy().to_string(),
            Err(_) => {
                // Fallback: current_dir + path
                let cwd = std::env::current_dir()
                    .map(|d| d.to_string_lossy().to_string())
                    .unwrap_or_default();
                let joined = std::path::Path::new(&cwd).join(&s);
                joined.to_string_lossy().to_string()
            }
        };
        // Get Path type from thread_local and create a new Path instance
        let path_type = PATH_TYPE
            .with(|cell| cell.borrow().clone())
            .ok_or_else(|| PyError::runtime_error("Path type not initialized".to_string()))?;
        let mut instance_dict = AttrMap::new();
        instance_dict.insert_str("_path", py_str(&result));
        Ok(PyObjectRef::new(PyObject::Instance {
            typ: path_type,
            dict: instance_dict,
        }))
    });

    // .write_text(data, encoding=None) -> number of characters written
    path_func!("write_text", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("write_text() missing data argument"));
        }
        let s = path_instance_str(&args[0]);
        let data = args[1].str();
        match std::fs::write(&s, data.as_bytes()) {
            Ok(_) => Ok(py_int(data.len() as i64)),
            Err(e) => Err(PyError::os_error_from_io(&e)),
        }
    });

    // .read_text(encoding=None) -> str
    path_func!("read_text", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("read_text() missing argument"));
        }
        let s = path_instance_str(&args[0]);
        match std::fs::read_to_string(&s) {
            Ok(data) => Ok(py_str(&data)),
            Err(e) => Err(PyError::os_error_from_io(&e)),
        }
    });

    // .write_bytes(data) -> None
    path_func!("write_bytes", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("write_bytes() missing data argument"));
        }
        let s = path_instance_str(&args[0]);
        let data = match &*args[1].borrow() {
            PyObject::Bytes(b) => b.clone(),
            _ => args[1].str().into_bytes(),
        };
        match std::fs::write(&s, &data) {
            Ok(_) => Ok(py_none()),
            Err(e) => Err(PyError::os_error_from_io(&e)),
        }
    });

    // .read_bytes() -> bytes
    path_func!("read_bytes", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("read_bytes() missing argument"));
        }
        let s = path_instance_str(&args[0]);
        match std::fs::read(&s) {
            Ok(data) => Ok(PyObjectRef::imm(PyObject::Bytes(data))),
            Err(e) => Err(PyError::os_error_from_io(&e)),
        }
    });

    fn new_path_instance(s: &str) -> PyResult<PyObjectRef> {
        let path_type = PATH_TYPE
            .with(|cell| cell.borrow().clone())
            .ok_or_else(|| PyError::runtime_error("Path type not initialized".to_string()))?;
        let mut instance_dict = AttrMap::new();
        instance_dict.insert_str("_path", py_str(s));
        Ok(PyObjectRef::new(PyObject::Instance {
            typ: path_type,
            dict: instance_dict,
        }))
    }

    // A trailing dict argument may be packed kwargs (see the calling-
    // convention gotcha in CLAUDE.md) — pull a named bool option out of it
    // (or a plain trailing positional bool, for the rare call site that
    // used one) without misreading either.
    fn bool_option(args: &[PyObjectRef], name: &str) -> bool {
        if let Some(last) = args.last() {
            if let PyObject::Dict(d) = &*last.borrow() {
                if let Ok(Some(v)) = d.get(&py_str(name)) {
                    return v.truthy();
                }
            }
        }
        false
    }

    // .mkdir(mode=0o777, parents=False, exist_ok=False)
    path_func!("mkdir", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("mkdir() missing argument"));
        }
        let s = path_instance_str(&args[0]);
        let parents = bool_option(args, "parents");
        let exist_ok = bool_option(args, "exist_ok");
        let path = std::path::Path::new(&s);
        let result = if parents {
            std::fs::create_dir_all(path)
        } else {
            std::fs::create_dir(path)
        };
        match result {
            Ok(()) => Ok(py_none()),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && exist_ok => Ok(py_none()),
            Err(e) => Err(PyError::os_error_from_io(&e)),
        }
    });

    // .touch(mode=0o666, exist_ok=True)
    path_func!("touch", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("touch() missing argument"));
        }
        let s = path_instance_str(&args[0]);
        let path = std::path::Path::new(&s);
        if path.exists() {
            // exist_ok defaults to True in real pathlib; only raise if the
            // caller explicitly passed exist_ok=False.
            let explicit_no = args.iter().any(|a| {
                if let PyObject::Dict(d) = &*a.borrow() {
                    matches!(d.get(&py_str("exist_ok")), Ok(Some(v)) if !v.truthy())
                } else {
                    false
                }
            });
            if explicit_no {
                return Err(PyError::os_error_from_io(&std::io::Error::from(
                    std::io::ErrorKind::AlreadyExists,
                )));
            }
            return Ok(py_none());
        }
        match std::fs::File::create(path) {
            Ok(_) => Ok(py_none()),
            Err(e) => Err(PyError::os_error_from_io(&e)),
        }
    });

    // .unlink(missing_ok=False)
    path_func!("unlink", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("unlink() missing argument"));
        }
        let s = path_instance_str(&args[0]);
        let missing_ok = bool_option(args, "missing_ok");
        match std::fs::remove_file(&s) {
            Ok(()) => Ok(py_none()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && missing_ok => Ok(py_none()),
            Err(e) => Err(PyError::os_error_from_io(&e)),
        }
    });

    // .rmdir()
    path_func!("rmdir", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("rmdir() missing argument"));
        }
        let s = path_instance_str(&args[0]);
        std::fs::remove_dir(&s).map(|_| py_none()).map_err(|e| PyError::os_error_from_io(&e))
    });

    // .chmod(mode)
    path_func!("chmod", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("chmod() missing mode argument"));
        }
        let s = path_instance_str(&args[0]);
        let mode = args[1].as_i64().unwrap_or(0o755) as u32;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(mode);
            std::fs::set_permissions(&s, perms).map_err(|e| PyError::os_error_from_io(&e))?;
        }
        Ok(py_none())
    });

    // .stat() -> stat_result-like module (mirrors os.stat's own return shape)
    path_func!("stat", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("stat() missing argument"));
        }
        let s = path_instance_str(&args[0]);
        match std::fs::metadata(&s) {
            Ok(meta) => Ok(create_module(
                "stat_result",
                crate::modules::stat_to_dict(&meta),
            )),
            Err(e) => Err(PyError::os_error_from_io(&e)),
        }
    });

    // .with_suffix(suffix) -> new Path with the extension replaced
    path_func!("with_suffix", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("with_suffix() missing argument"));
        }
        let s = path_instance_str(&args[0]);
        let suffix = args[1].str();
        let path = std::path::Path::new(&s);
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let new_name = format!("{}{}", stem, suffix);
        let result = match path.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => {
                parent.join(&new_name).to_string_lossy().to_string()
            }
            _ => new_name,
        };
        new_path_instance(&result)
    });

    // .relative_to(other) -> new Path
    path_func!("relative_to", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("relative_to() missing argument"));
        }
        let s = path_instance_str(&args[0]);
        let other = path_instance_str(&args[1]);
        match std::path::Path::new(&s).strip_prefix(&other) {
            Ok(rel) => new_path_instance(&rel.to_string_lossy()),
            Err(_) => Err(PyError::value_error(format!(
                "{:?} is not in the subpath of {:?}",
                s, other
            ))),
        }
    });

    // .as_posix() -> str with '/' separators
    path_func!("as_posix", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("as_posix() missing argument"));
        }
        let s = path_instance_str(&args[0]);
        Ok(py_str(&s.replace('\\', "/")))
    });

    // .open(mode='r', ...) -> file object (delegates to the builtin open())
    path_func!("open", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("open() missing argument"));
        }
        let s = path_instance_str(&args[0]);
        let mut open_args = vec![py_str(&s)];
        open_args.extend_from_slice(&args[1..]);
        crate::object::builtin_open(&open_args)
    });

    // .is_absolute() -> bool
    path_func!("is_absolute", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("is_absolute() missing argument"));
        }
        let s = path_instance_str(&args[0]);
        Ok(py_bool(std::path::Path::new(&s).is_absolute()))
    });

    // Small manual glob-style matcher (`*`/`?`), no external regex crate
    // needed — same subset real `fnmatch` supports for these callers.
    fn glob_component_match(name: &str, pattern: &str) -> bool {
        fn rec(n: &[char], p: &[char]) -> bool {
            match p.first() {
                None => n.is_empty(),
                Some('*') => rec(n, &p[1..]) || (!n.is_empty() && rec(&n[1..], p)),
                Some('?') => !n.is_empty() && rec(&n[1..], &p[1..]),
                Some(c) => n.first() == Some(c) && rec(&n[1..], &p[1..]),
            }
        }
        let n: Vec<char> = name.chars().collect();
        let p: Vec<char> = pattern.chars().collect();
        rec(&n, &p)
    }

    // .match(pattern) -> bool (matches trailing path components against
    // the pattern's own components, real pathlib semantics)
    path_func!("match", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("match() missing pattern argument"));
        }
        let s = path_instance_str(&args[0]);
        let pattern = args[1].str();
        let path_parts: Vec<&str> = s.split('/').filter(|p| !p.is_empty()).collect();
        let pat_parts: Vec<&str> = pattern.split('/').filter(|p| !p.is_empty()).collect();
        if pat_parts.len() > path_parts.len() {
            return Ok(py_bool(false));
        }
        let offset = path_parts.len() - pat_parts.len();
        let matched = pat_parts
            .iter()
            .zip(&path_parts[offset..])
            .all(|(pat, name)| glob_component_match(name, pat));
        Ok(py_bool(matched))
    });

    // .parts -> tuple of path components (property getter)
    {
        let getter = PyObjectRef::new(PyObject::BuiltinFunction {
            name: "parts".to_string(),
            func: |args| {
                if args.is_empty() {
                    return Err(PyError::type_error("parts getter missing argument"));
                }
                let s = path_instance_str(&args[0]);
                let path = std::path::Path::new(&s);
                let mut parts = vec![];
                for comp in path.components() {
                    parts.push(py_str(&comp.as_os_str().to_string_lossy()));
                }
                Ok(py_tuple(parts))
            },
        });
        path_type_dict.insert_str(
            "parts",
            PyObjectRef::new(PyObject::Property(Box::new(PropertyData {
                getter: Some(getter),
                setter: None,
                deleter: None,
                doc: None,
            }))),
        );
    }

    // .__fspath__() -> str (so `open(path)`/`os.*` functions taking
    // PathLike objects can consume a Path directly).
    path_func!("__fspath__", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("__fspath__() missing argument"));
        }
        Ok(py_str(&path_instance_str(&args[0])))
    });

    // .iterdir() -> iterator of child Paths
    path_func!("iterdir", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("iterdir() missing argument"));
        }
        let s = path_instance_str(&args[0]);
        let mut entries = vec![];
        let read = std::fs::read_dir(&s).map_err(|e| PyError::os_error_from_io(&e))?;
        for entry in read.flatten() {
            entries.push(new_path_instance(&entry.path().to_string_lossy())?);
        }
        crate::object::builtin_iter(&[py_list(entries)])
    });

    // .glob(pattern) -> list[Path] (shallow: only "*"/plain-name patterns
    // and a leading "**/" are handled; enough for the common recursive-glob
    // idiom, not full fnmatch-on-every-component).
    fn collect_glob(base: &std::path::Path, pattern: &str, recursive_all: bool) -> Vec<std::path::PathBuf> {
        let mut out = vec![];
        let entries = match std::fs::read_dir(base) {
            Ok(e) => e,
            Err(_) => return out,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            let matches = pattern == "*" || pattern == "**" || name == pattern;
            if matches {
                out.push(path.clone());
            }
            if recursive_all && path.is_dir() {
                out.extend(collect_glob(&path, pattern, recursive_all));
            }
        }
        out
    }

    path_func!("rglob", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("rglob() missing pattern argument"));
        }
        let s = path_instance_str(&args[0]);
        let pattern = args[1].str();
        let base = std::path::Path::new(&s);
        let mut results = collect_glob(base, &pattern, true);
        results.sort();
        let mut out = vec![];
        for p in results {
            out.push(new_path_instance(&p.to_string_lossy())?);
        }
        Ok(py_list(out))
    });

    path_func!("glob", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("glob() missing pattern argument"));
        }
        let s = path_instance_str(&args[0]);
        let pattern = args[1].str();
        let base = std::path::Path::new(&s);
        let mut results = collect_glob(base, &pattern, false);
        results.sort();
        let mut out = vec![];
        for p in results {
            out.push(new_path_instance(&p.to_string_lossy())?);
        }
        Ok(py_list(out))
    });

    // __eq__ / __hash__ / __lt__: compare/hash by the normalized path
    // string. Needed for `target in files_to_add` (a dict keyed by Path)
    // in `zipapp.create_archive` and for `sorted(source.rglob('*'))`.
    path_func!("__eq__", |args| {
        if args.len() < 2 {
            return Ok(py_bool(false));
        }
        if !matches!(&*args[1].borrow(), PyObject::Instance { .. }) {
            return Ok(py_bool(false));
        }
        let a = path_instance_str(&args[0]);
        let b = path_instance_str(&args[1]);
        Ok(py_bool(a == b))
    });

    path_func!("__hash__", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("__hash__() missing argument"));
        }
        let s = path_instance_str(&args[0]);
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        s.hash(&mut hasher);
        Ok(py_int(hasher.finish() as i64))
    });

    path_func!("__lt__", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("__lt__() missing argument"));
        }
        let a = path_instance_str(&args[0]);
        let b = path_instance_str(&args[1]);
        Ok(py_bool(a < b))
    });

    // Create the Path Type object
    let path_type = PyObjectRef::new(PyObject::Type {
        name: "Path".to_string(),
        dict: Box::new(str_map_to_typedict(path_type_dict)),
        bases: vec![],
        mro: vec![],
    });

    // Store Path type in thread_local for joinpath/absolute to use
    PATH_TYPE.with(|cell| {
        *cell.borrow_mut() = Some(path_type.clone());
    });

    let mut d = HashMap::new();
    d.insert_str("Path", path_type);
    d
}

// Moved here from object.rs (was under a "=== SHELVE MODULE ===" banner in
// the monolithic object.rs — see the file-splitting refactor's memory
// entry for context).
// Shelf class backed by a dict. open(filename) -> Shelf instance.

/// Extract the _data dict from a Shelf Instance (args[0]).
fn shelf_get_data(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error("method requires self"));
    }
    match &*args[0].borrow() {
        PyObject::Instance { dict, .. } => match dict.get("_data") {
            Some(data) => Ok(data.clone()),
            None => Err(PyError::runtime_error(
                "Shelf instance corrupted: missing _data",
            )),
        },
        _ => Err(PyError::type_error("expected Shelf instance")),
    }
}

fn shelf_close(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let _ = args;
    Ok(py_none())
}

fn shelf_sync(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let _ = args;
    Ok(py_none())
}

fn shelf_get(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    // args[0] = self, args[1] = key, args[2] = default (optional)
    if args.len() < 2 {
        return Err(PyError::type_error(
            "get() takes at least 2 arguments (self, key)",
        ));
    }
    let data = shelf_get_data(args)?;
    let key = args[1].str();
    let data_ref = data.borrow();
    if let PyObject::Dict(ref d) = &*data_ref {
        let py_key = py_str(&key);
        match d.get(&py_key)? {
            Some(val) => Ok(val),
            None => {
                if args.len() > 2 {
                    Ok(args[2].clone())
                } else {
                    Ok(py_none())
                }
            }
        }
    } else {
        Ok(py_none())
    }
}

fn shelf_keys(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let data = shelf_get_data(args)?;
    let data_ref = data.borrow();
    if let PyObject::Dict(ref d) = &*data_ref {
        let ks = d.keys();
        Ok(PyObjectRef::new(PyObject::List(ks)))
    } else {
        Ok(PyObjectRef::new(PyObject::List(vec![])))
    }
}

fn shelf_values(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let data = shelf_get_data(args)?;
    let data_ref = data.borrow();
    if let PyObject::Dict(ref d) = &*data_ref {
        let vs = d.values();
        Ok(PyObjectRef::new(PyObject::List(vs)))
    } else {
        Ok(PyObjectRef::new(PyObject::List(vec![])))
    }
}

fn shelf_items(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let data = shelf_get_data(args)?;
    let data_ref = data.borrow();
    if let PyObject::Dict(ref d) = &*data_ref {
        let pairs: Vec<PyObjectRef> = d
            .items()
            .into_iter()
            .map(|(k, v)| PyObjectRef::new(PyObject::Tuple(vec![k, v])))
            .collect();
        Ok(PyObjectRef::new(PyObject::List(pairs)))
    } else {
        Ok(PyObjectRef::new(PyObject::List(vec![])))
    }
}

// __len__(self) -> int (for len())
fn shelf_len(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let data = shelf_get_data(args)?;
    let data_ref = data.borrow();
    if let PyObject::Dict(ref d) = &*data_ref {
        Ok(py_int(d.len() as i64))
    } else {
        Ok(py_int(0))
    }
}

// __contains__(self, key) -> bool (for 'key in shelf')
fn shelf_contains(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Err(PyError::type_error(
            "__contains__() takes at least 2 arguments (self, key)",
        ));
    }
    let data = shelf_get_data(args)?;
    let key = args[1].str();
    let data_ref = data.borrow();
    if let PyObject::Dict(ref d) = &*data_ref {
        let py_key = py_str(&key);
        Ok(py_bool(d.contains(&py_key)?))
    } else {
        Ok(py_bool(false))
    }
}

// __repr__(self) -> str
fn shelf_repr(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let data = shelf_get_data(args)?;
    let data_ref = data.borrow();
    if let PyObject::Dict(ref d) = &*data_ref {
        Ok(py_str(&format!("Shelf({} items)", d.len())))
    } else {
        Ok(py_str("Shelf(0 items)"))
    }
}

// __getitem__(self, key) -> value (for shelf[key])
fn shelf_getitem(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Err(PyError::type_error(
            "__getitem__() takes at least 2 arguments (self, key)",
        ));
    }
    let data = shelf_get_data(args)?;
    let key = args[1].str();
    let data_ref = data.borrow();
    if let PyObject::Dict(ref d) = &*data_ref {
        let py_key = py_str(&key);
        match d.get(&py_key)? {
            Some(val) => Ok(val),
            None => Err(PyError::key_error(format!("'{}'", key))),
        }
    } else {
        Err(PyError::key_error(format!("'{}'", key)))
    }
}

// __setitem__(self, key, value) (for shelf[key] = value)
fn shelf_setitem(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 3 {
        return Err(PyError::type_error(
            "__setitem__() takes at least 3 arguments (self, key, value)",
        ));
    }
    let data = shelf_get_data(args)?;
    let key = args[1].str();
    {
        let mut data_mut = data.borrow_mut();
        if let PyObject::Dict(ref mut d) = &mut *data_mut {
            d.set(py_str(&key), args[2].clone())?;
        }
    }
    Ok(py_none())
}

// __delitem__(self, key) (for del shelf[key])
fn shelf_delitem(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Err(PyError::type_error(
            "__delitem__() takes at least 2 arguments (self, key)",
        ));
    }
    let data = shelf_get_data(args)?;
    let key = args[1].str();
    {
        let mut data_mut = data.borrow_mut();
        if let PyObject::Dict(ref mut d) = &mut *data_mut {
            let py_key = py_str(&key);
            d.remove(&py_key)?;
        }
    }
    Ok(py_none())
}

pub fn shelf_open(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error(
            "open() takes at least 1 argument (filename)",
        ));
    }
    let filename = args[0].str();

    // Internal data dict
    let data_dict = py_dict();

    // Instance dict with field and methods
    let mut inst_dict = AttrMap::new();
    inst_dict.insert("_data".to_string(), data_dict);
    inst_dict.insert("filename".to_string(), py_str(&filename));

    inst_dict.insert(
        "close".to_string(),
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "close".to_string(),
            func: shelf_close,
        }),
    );
    inst_dict.insert(
        "sync".to_string(),
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "sync".to_string(),
            func: shelf_sync,
        }),
    );
    inst_dict.insert(
        "get".to_string(),
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "get".to_string(),
            func: shelf_get,
        }),
    );
    inst_dict.insert(
        "keys".to_string(),
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "keys".to_string(),
            func: shelf_keys,
        }),
    );
    inst_dict.insert(
        "values".to_string(),
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "values".to_string(),
            func: shelf_values,
        }),
    );
    inst_dict.insert(
        "items".to_string(),
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "items".to_string(),
            func: shelf_items,
        }),
    );

    // Type dict with dunder methods (used by py_getitem/py_setitem dispatch)
    let mut type_dict = HashMap::new();
    type_dict.insert(
        "__getitem__".to_string(),
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__getitem__".to_string(),
            func: shelf_getitem,
        }),
    );
    type_dict.insert(
        "__setitem__".to_string(),
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__setitem__".to_string(),
            func: shelf_setitem,
        }),
    );
    type_dict.insert(
        "__delitem__".to_string(),
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__delitem__".to_string(),
            func: shelf_delitem,
        }),
    );
    type_dict.insert(
        "__len__".to_string(),
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__len__".to_string(),
            func: shelf_len,
        }),
    );
    type_dict.insert(
        "__contains__".to_string(),
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__contains__".to_string(),
            func: shelf_contains,
        }),
    );
    type_dict.insert(
        "__repr__".to_string(),
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__repr__".to_string(),
            func: shelf_repr,
        }),
    );

    // Build Shelf type
    let shelf_type = PyObjectRef::new(PyObject::Type {
        name: "Shelf".to_string(),
        dict: Box::new(str_map_to_typedict(type_dict)),
        bases: vec![],
        // MRO includes self so __getitem__ lookup works
        mro: vec![],
    });

    let instance = PyObjectRef::new(PyObject::Instance {
        typ: shelf_type,
        dict: inst_dict,
    });

    Ok(instance)
}

