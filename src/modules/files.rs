use crate::object::*;
use std::collections::HashMap;

pub fn create_glob_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! glob_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    fn glob_match(name: &str, pattern: &str) -> bool {
        let re_str = format!("^{}$", pattern.replace(".", "\\.").replace("?", ".").replace("*", ".*"));
        regex::Regex::new(&re_str).map(|re| re.is_match(name)).unwrap_or(false)
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
                let full = if prefix.is_empty() { name.clone() } else { format!("{}/{}", prefix, name) };
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

        let start = if is_absolute { std::path::Path::new("/") } else { std::path::Path::new(".") };

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
    d
}

pub fn create_fnmatch_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! fnmatch_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    fn fnmatch_match(name: &str, pattern: &str) -> bool {
        let mut re_str = String::from("^");
        for ch in pattern.chars() {
            match ch {
                '.' => re_str.push_str("\\."),
                '*' => re_str.push_str(".*"),
                '?' => re_str.push('.'),
                other => re_str.push(other),
            }
        }
        re_str.push('$');
        regex::Regex::new(&re_str).map(|re| re.is_match(name)).unwrap_or(false)
    }

    fnmatch_func!("fnmatch", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("fnmatch() takes exactly 2 arguments"));
        }
        let name = args[0].str();
        let pattern = args[1].str();
        Ok(py_bool(fnmatch_match(&name, &pattern)))
    });
    d
}

pub fn create_tempfile_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! temp_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    // Simple random suffix generator using /dev/urandom or fallback
    fn random_suffix(len: usize) -> String {
        use std::time::{SystemTime, UNIX_EPOCH};
        // Mix time-based and random characters
        let ts = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
        let mut result = String::with_capacity(len);
        let chars = b"abcdefghijklmnopqrstuvwxyz0123456789";
        let mut seed = ts;
        for _ in 0..len {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let idx = (seed as usize) % chars.len();
            result.push(chars[idx] as char);
        }
        result
    }

    temp_func!("mkstemp", |_| {
        // Try up to 10 times to create a unique temp file
        for _ in 0..10 {
            let suffix = random_suffix(8);
            let name = format!("/tmp/tmp{}", suffix);
            match std::fs::File::create_new(&name) {
                Ok(file) => {
                    use std::os::fd::AsRawFd;
                    let fd = file.as_raw_fd();
                    // Return (fd, name) as a tuple
                    return Ok(py_tuple(vec![py_int(fd as i64), py_str(&name)]));
                }
                Err(_) => continue,
            }
        }
        Err(PyError::runtime_error("could not create temporary file"))
    });

    temp_func!("mkdtemp", |_| {
        for _ in 0..10 {
            let suffix = random_suffix(8);
            let name = format!("/tmp/tmp{}", suffix);
            match std::fs::create_dir(&name) {
                Ok(()) => return Ok(py_str(&name)),
                Err(_) => continue,
            }
        }
        Err(PyError::runtime_error("could not create temporary directory"))
    });

    // Add temporary directory path
    d.insert("tempdir".to_string(), py_str("/tmp"));
    d
}

pub fn create_shutil_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! shutil_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }
    shutil_func!("copy", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("copy() requires 2 arguments (src, dst)"));
        }
        let src = args[0].str();
        let dst = args[1].str();
        match std::fs::copy(&src, &dst) {
            Ok(_) => Ok(py_str(&dst)),
            Err(e) => Err(PyError::OsError(format!("copy error: {}", e))),
        }
    });

    shutil_func!("rmtree", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("rmtree() requires 1 argument (path)"));
        }
        let path = args[0].str();
        match std::fs::remove_dir_all(&path) {
            Ok(()) => Ok(py_none()),
            Err(e) => Err(PyError::OsError(format!("rmtree error: {}", e))),
        }
    });

    shutil_func!("move", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("move() requires 2 arguments (src, dst)"));
        }
        let src = args[0].str();
        let dst = args[1].str();
        match std::fs::rename(&src, &dst) {
            Ok(()) => Ok(py_str(&dst)),
            Err(e) => Err(PyError::OsError(format!("move error: {}", e))),
        }
    });
    d
}

pub fn create_gzip_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! gz_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    gz_func!("open", |args| {
        if args.len() < 1 || args.len() > 2 {
            return Err(PyError::type_error("open() takes 1-2 arguments (filename, [mode])"));
        }
        let filename = args[0].borrow().str();
        let mode = if args.len() > 1 { args[1].borrow().str() } else { "rb".to_string() };
        if mode.contains('w') || mode.contains('x') || mode.contains('a') {
            match std::fs::File::create(&filename) {
                Ok(_) => Ok(py_none()),
                Err(e) => Err(PyError::runtime_error(format!("gzip.open: cannot create '{}': {}", filename, e))),
            }
        } else {
            match std::fs::File::open(&filename) {
                Ok(_) => Ok(py_none()),
                Err(e) => Err(PyError::runtime_error(format!("gzip.open: cannot open '{}': {}", filename, e))),
            }
        }
    });

    gz_func!("compress", |args| {
        if args.len() != 1 { return Err(PyError::type_error("compress() takes exactly one argument")); }
        let data = args[0].borrow();
        let bytes = match &*data {
            PyObject::Bytes(b) => b.clone(),
            PyObject::Str(s) => s.as_bytes().to_vec(),
            _ => return Err(PyError::type_error("compress() argument must be bytes or str")),
        };
        // Build a minimal gzip stream: header (10 bytes) + stored data + crc32 + size
        let mut result = Vec::with_capacity(bytes.len() + 18);
        // ID1, ID2 (magic)
        result.push(0x1f);
        result.push(0x8b);
        // CM = deflate (8)
        result.push(8);
        // FLG = 0
        result.push(0);
        // MTIME = 0 (4 bytes)
        result.extend_from_slice(&[0u8; 4]);
        // XFL = 0
        result.push(0);
        // OS = 255 (unknown)
        result.push(255);
        // Store raw data (no compression)
        result.extend_from_slice(&bytes);
        // CRC32 (4 bytes)
        let crc = gzip_crc32(&bytes);
        result.extend_from_slice(&crc.to_le_bytes());
        // Original size (4 bytes)
        result.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        Ok(PyObjectRef::new(PyObject::Bytes(result)))
    });

    d
}

pub fn create_tarfile_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! tar_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    tar_func!("open", |args| {
        if args.len() < 1 { return Err(PyError::type_error("tarfile.open() takes at least 1 argument (name)")); }
        let _name = args[0].borrow().str();
        // Return an Instance with getnames() and extractall() methods
        let mut inst_dict = HashMap::new();
        inst_dict.insert("name".to_string(), py_str(&_name));
        inst_dict.insert("getnames".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
            name: "getnames".to_string(),
            func: |_args| Ok(py_list(vec![])),
        }));
        inst_dict.insert("extractall".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
            name: "extractall".to_string(),
            func: |_args| Ok(py_none()),
        }));
        Ok(PyObjectRef::new(PyObject::Instance {
            typ: PyObjectRef::new(PyObject::Module {
                name: "tarfile.TarFile".to_string(),
                dict: HashMap::new(),
            }),
            dict: inst_dict,
        }))
    });

    d
}

pub fn create_pathlib_dict() -> HashMap<String, PyObjectRef> {
    let mut path_type_dict = HashMap::new();

    macro_rules! path_func {
        ($name:expr, $func:expr) => {
            path_type_dict.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    // Helper to get the path string from a Path instance
    fn path_instance_str(instance: &PyObjectRef) -> String {
        instance.borrow().get_attribute("_path")
            .map(|v| v.str())
            .unwrap_or_default()
    }

    // __str__: str(path) returns the path string
    path_func!("__str__", |args| {
        if args.is_empty() { return Err(PyError::type_error("__str__() missing argument")); }
        Ok(py_str(&path_instance_str(&args[0])))
    });

    // __repr__: repr(path)
    path_func!("__repr__", |args| {
        if args.is_empty() { return Err(PyError::type_error("__repr__() missing argument")); }
        let s = path_instance_str(&args[0]);
        Ok(py_str(&format!("PurePosixPath('{}')", s)))
    });

    // __init__: Path(path_str) stores the path string
    path_func!("__init__", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("__init__() missing path argument"));
        }
        let path_val = if args.len() > 1 {
            py_str(&args[1].str())
        } else {
            py_str(".")
        };
        if let PyObject::Instance { dict, .. } = &mut *args[0].borrow_mut() {
            dict.insert("_path".to_string(), path_val);
        }
        Ok(py_none())
    });

    // .parent -> dirname (property getter)
    {
        let getter = PyObjectRef::new(PyObject::BuiltinFunction {
            name: "parent".to_string(),
            func: |args| {
                if args.is_empty() {
                    return Err(PyError::type_error("parent getter missing argument"));
                }
                let s = path_instance_str(&args[0]);
                let parent = std::path::Path::new(&s).parent()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default();
                Ok(py_str(&parent))
            },
        });
        path_type_dict.insert("parent".to_string(), PyObjectRef::new(PyObject::Property {
            getter: Some(getter),
            setter: None,
            deleter: None,
            doc: None,
        }));
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
                let name = std::path::Path::new(&s).file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                Ok(py_str(&name))
            },
        });
        path_type_dict.insert("name".to_string(), PyObjectRef::new(PyObject::Property {
            getter: Some(getter),
            setter: None,
            deleter: None,
            doc: None,
        }));
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
                let suffix = std::path::Path::new(&s).extension()
                    .map(|e| format!(".{}", e.to_string_lossy()))
                    .unwrap_or_default();
                Ok(py_str(&suffix))
            },
        });
        path_type_dict.insert("suffix".to_string(), PyObjectRef::new(PyObject::Property {
            getter: Some(getter),
            setter: None,
            deleter: None,
            doc: None,
        }));
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
                let stem = std::path::Path::new(&s).file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default();
                Ok(py_str(&stem))
            },
        });
        path_type_dict.insert("stem".to_string(), PyObjectRef::new(PyObject::Property {
            getter: Some(getter),
            setter: None,
            deleter: None,
            doc: None,
        }));
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
        let path_type = PATH_TYPE.with(|cell| {
            cell.borrow().clone()
        }).ok_or_else(|| PyError::runtime_error("Path type not initialized".to_string()))?;
        let mut instance_dict = HashMap::new();
        instance_dict.insert("_path".to_string(), py_str(&result));
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
        let path_type = PATH_TYPE.with(|cell| {
            cell.borrow().clone()
        }).ok_or_else(|| PyError::runtime_error("Path type not initialized".to_string()))?;
        let mut instance_dict = HashMap::new();
        instance_dict.insert("_path".to_string(), py_str(&result));
        Ok(PyObjectRef::new(PyObject::Instance {
            typ: path_type,
            dict: instance_dict,
        }))
    });

    // Create the Path Type object
    let path_type = PyObjectRef::new(PyObject::Type {
        name: "Path".to_string(),
        dict: path_type_dict,
        bases: vec![],
        mro: vec![],
    });

    // Store Path type in thread_local for joinpath/absolute to use
    PATH_TYPE.with(|cell| {
        *cell.borrow_mut() = Some(path_type.clone());
    });

    let mut d = HashMap::new();
    d.insert("Path".to_string(), path_type);
    d
}

pub fn create_zipfile_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    d.insert("ZipFile".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "ZipFile".to_string(),
        func: zipfile_constructor,
    }));
    d
}

pub fn create_shelve_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    d.insert("open".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "open".to_string(), func: shelf_open }));
    d.insert("Shelf".to_string(), py_str("Shelf"));
    d
}

pub fn create_linecache_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! lc_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    lc_func!("getline", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("getline() requires at least 2 arguments (filename, lineno)"));
        }
        Ok(py_str(""))
    });

    lc_func!("clearcache", |_| {
        Ok(py_none())
    });

    lc_func!("checkcache", |_| {
        Ok(py_none())
    });

    d
}

use std::rc::Rc;
use std::cell::RefCell;
use std::sync::atomic::{AtomicI64, Ordering};
use num_traits::ToPrimitive;
use crate::bytecode::{needs_arg, CodeObject};