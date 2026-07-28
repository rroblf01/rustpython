use crate::object::*;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::rc::Rc;
use std::cell::RefCell;
use num_traits::ToPrimitive;

// Moved here from object.rs (was under a "---- pathlib module ----" banner in
// the monolithic object.rs, alongside other misplaced stdlib-module code —
// see the file-splitting refactor's memory entry for context).
thread_local! {
    pub static PATH_TYPE: std::cell::RefCell<Option<PyObjectRef>> = std::cell::RefCell::new(None);
}

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

    // `fnmatch.translate(pattern)` returns the REGEX SOURCE STRING a glob
    // pattern compiles to (real CPython: `(?s:REGEX)\Z`) — was missing
    // entirely. Real code uses this to build its OWN compiled regex (e.g.
    // combining several glob patterns into one alternation), rather than
    // calling `fnmatch()` per name repeatedly.
    fn fnmatch_translate_str(pattern: &str) -> String {
        let mut re_str = String::new();
        for ch in pattern.chars() {
            match ch {
                '.' => re_str.push_str("\\."),
                '*' => re_str.push_str(".*"),
                '?' => re_str.push('.'),
                '(' | ')' | '[' | ']' | '{' | '}' | '+' | '^' | '$' | '|' | '\\' => {
                    re_str.push('\\');
                    re_str.push(ch);
                }
                other => re_str.push(other),
            }
        }
        format!("(?s:{})\\Z", re_str)
    }

    fnmatch_func!("fnmatch", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("fnmatch() takes exactly 2 arguments"));
        }
        let name = args[0].str();
        let pattern = args[1].str();
        Ok(py_bool(fnmatch_match(&name, &pattern)))
    });
    // fnmatchcase(name, pattern) — always case-sensitive (unlike `fnmatch`,
    // which normalizes case on case-insensitive filesystems via
    // os.path.normcase). Our `fnmatch_match` never does that normalization
    // to begin with, so this is simply the same matcher under its other
    // real name — was missing entirely (`from fnmatch import fnmatchcase`,
    // real code in CPython's own `unittest.util`).
    fnmatch_func!("fnmatchcase", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("fnmatchcase() takes exactly 2 arguments"));
        }
        let name = args[0].str();
        let pattern = args[1].str();
        Ok(py_bool(fnmatch_match(&name, &pattern)))
    });
    fnmatch_func!("translate", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("translate() takes exactly 1 argument"));
        }
        Ok(py_str(&fnmatch_translate_str(&args[0].str())))
    });
    d
}

pub fn create_shutil_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! shutil_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
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
            } else { (80, 24) }
        } else { (80, 24) };
        let columns = std::env::var("COLUMNS").ok().and_then(|s| s.parse::<i64>().ok()).unwrap_or(fallback_cols);
        let lines = std::env::var("LINES").ok().and_then(|s| s.parse::<i64>().ok()).unwrap_or(fallback_lines);
        let typ = PyObjectRef::new(PyObject::Type { name: "os.terminal_size".to_string(), dict: Box::new(str_map_to_typedict(HashMap::new())), bases: vec![], mro: vec![] });
        let mut dict = AttrMap::new();
        dict.insert_str("columns", py_int(columns));
        dict.insert_str("lines", py_int(lines));
        Ok(PyObjectRef::new(PyObject::Instance { typ, dict }))
    });

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

    shutil_func!("copymode", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("copymode() requires 2 arguments (src, dst)"));
        }
        let src = args[0].str();
        let dst = args[1].str();
        let perms = std::fs::metadata(&src).map_err(|e| PyError::OsError(format!("{}", e)))?.permissions();
        std::fs::set_permissions(&dst, perms).map_err(|e| PyError::OsError(format!("{}", e)))?;
        Ok(py_none())
    });

    shutil_func!("copystat", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("copystat() requires 2 arguments (src, dst)"));
        }
        let src = args[0].str();
        let dst = args[1].str();
        let perms = std::fs::metadata(&src).map_err(|e| PyError::OsError(format!("{}", e)))?.permissions();
        std::fs::set_permissions(&dst, perms).map_err(|e| PyError::OsError(format!("{}", e)))?;
        Ok(py_none())
    });
    d
}

fn gzip_bytes_arg(obj: &PyObjectRef) -> PyResult<Vec<u8>> {
    match &*obj.borrow() {
        PyObject::Bytes(b) => Ok(b.clone()),
        PyObject::ByteArray(b) => Ok(b.clone()),
        PyObject::Str(s) => Ok(s.as_bytes().to_vec()),
        _ => Err(PyError::type_error("a bytes-like object is required")),
    }
}

/// Pull `compresslevel`/`mtime` out of trailing args, which may be a bare
/// positional int (compresslevel) and/or a trailing kwargs dict (since
/// `call_function` packs keyword arguments into one dict appended after
/// positionals).
fn gzip_parse_level_mtime(rest: &[PyObjectRef]) -> (u32, Option<u32>) {
    let mut compresslevel: u32 = 9;
    let mut mtime: Option<u32> = None;
    for a in rest {
        match &*a.borrow() {
            PyObject::Int(i) => {
                if let Some(n) = i.to_i64() { compresslevel = n as u32; }
            }
            PyObject::Dict(dct) => {
                if let Ok(Some(v)) = dct.get(&py_str("compresslevel")) {
                    if let Some(n) = v.as_i64() { compresslevel = n as u32; }
                }
                if let Ok(Some(v)) = dct.get(&py_str("mtime")) {
                    if let Some(n) = v.as_i64() { mtime = Some(n as u32); }
                }
            }
            _ => {}
        }
    }
    (compresslevel, mtime)
}

/// Build a `GzipFile`-like instance (used by both `gzip.open()` and
/// `gzip.GzipFile()`) following the BytesIO pattern: a fresh `Type` per
/// instance whose methods are `Closure`s capturing the shared native state
/// directly, rather than routing through the instance dict.
fn build_gzip_file(filename: &str, mode: &str, compresslevel: u32, mtime: Option<u32>, text: bool, encoding: &str) -> PyResult<PyObjectRef> {
    let writing = mode.contains('w') || mode.contains('a') || mode.contains('x');
    let mut type_dict: HashMap<String, PyObjectRef> = HashMap::new();

    type_dict.insert_str("__init__", PyObjectRef::new(PyObject::BuiltinFunction {
        name: "__init__".to_string(), func: |_: &[PyObjectRef]| Ok(py_none()),
    }));
    type_dict.insert_str("mode", py_str(mode));
    type_dict.insert_str("name", py_str(filename));

    if writing {
        let file = std::fs::File::options()
            .write(true)
            .create(true)
            .append(mode.contains('a'))
            .truncate(!mode.contains('a'))
            .open(filename)
            .map_err(|e| PyError::OsError(format!("{}", e)))?;
        let encoder = flate2::GzBuilder::new()
            .mtime(mtime.unwrap_or(0))
            .write(file, flate2::Compression::new(compresslevel.min(9)));
        let enc_rc: Rc<RefCell<Option<flate2::write::GzEncoder<std::fs::File>>>> = Rc::new(RefCell::new(Some(encoder)));

        let enc_write = enc_rc.clone();
        let encoding_owned = encoding.to_string();
        type_dict.insert_str("write", PyObjectRef::new(PyObject::Closure(Rc::new(move |args: &[PyObjectRef]| {
            if args.is_empty() { return Err(PyError::type_error("write() takes exactly one argument")); }
            let bytes = if text {
                args[0].str().into_bytes()
            } else {
                gzip_bytes_arg(&args[0])?
            };
            let mut slot = enc_write.borrow_mut();
            let enc = slot.as_mut().ok_or_else(|| PyError::value_error("I/O operation on closed file"))?;
            enc.write_all(&bytes).map_err(|e| PyError::OsError(format!("{}", e)))?;
            let _ = &encoding_owned;
            Ok(py_int(bytes.len() as i64))
        }))));

        let enc_flush = enc_rc.clone();
        type_dict.insert_str("flush", PyObjectRef::new(PyObject::Closure(Rc::new(move |_: &[PyObjectRef]| {
            if let Some(enc) = enc_flush.borrow_mut().as_mut() {
                enc.flush().map_err(|e| PyError::OsError(format!("{}", e)))?;
            }
            Ok(py_none())
        }))));

        let enc_close = enc_rc.clone();
        type_dict.insert_str("close", PyObjectRef::new(PyObject::Closure(Rc::new(move |_: &[PyObjectRef]| {
            if let Some(enc) = enc_close.borrow_mut().take() {
                enc.finish().map_err(|e| PyError::OsError(format!("{}", e)))?;
            }
            Ok(py_none())
        }))));
    } else {
        let file = std::fs::File::open(filename).map_err(|e| PyError::OsError(format!("{}", e)))?;
        let mut decoder = flate2::read::GzDecoder::new(file);
        let mut data = Vec::new();
        decoder.read_to_end(&mut data).map_err(|e| PyError::OsError(format!("{}", e)))?;
        let buf_rc = Rc::new(RefCell::new(data));
        let pos_rc = Rc::new(RefCell::new(0usize));
        let encoding_owned = encoding.to_string();

        let b_read = buf_rc.clone();
        let p_read = pos_rc.clone();
        let enc_read = encoding_owned.clone();
        type_dict.insert_str("read", PyObjectRef::new(PyObject::Closure(Rc::new(move |args: &[PyObjectRef]| {
            let data = b_read.borrow();
            let pos = (*p_read.borrow()).min(data.len());
            let end = if !args.is_empty() {
                args[0].as_i64().filter(|&n| n >= 0).map(|n| (pos + n as usize).min(data.len())).unwrap_or(data.len())
            } else {
                data.len()
            };
            let chunk = data[pos..end].to_vec();
            *p_read.borrow_mut() = end;
            if text {
                Ok(py_str(&decode_bytes(&chunk, &enc_read)))
            } else {
                Ok(PyObjectRef::imm(PyObject::Bytes(chunk)))
            }
        }))));

        let b_readline = buf_rc.clone();
        let p_readline = pos_rc.clone();
        let enc_readline = encoding_owned.clone();
        type_dict.insert_str("readline", PyObjectRef::new(PyObject::Closure(Rc::new(move |_: &[PyObjectRef]| {
            let data = b_readline.borrow();
            let pos = (*p_readline.borrow()).min(data.len());
            let remaining = &data[pos..];
            let end = remaining.iter().position(|&c| c == b'\n').map(|i| i + 1).unwrap_or(remaining.len());
            let chunk = remaining[..end].to_vec();
            *p_readline.borrow_mut() = pos + end;
            if text {
                Ok(py_str(&decode_bytes(&chunk, &enc_readline)))
            } else {
                Ok(PyObjectRef::imm(PyObject::Bytes(chunk)))
            }
        }))));

        let b_readlines = buf_rc.clone();
        let p_readlines = pos_rc.clone();
        let enc_readlines = encoding_owned.clone();
        type_dict.insert_str("readlines", PyObjectRef::new(PyObject::Closure(Rc::new(move |_: &[PyObjectRef]| {
            let data = b_readlines.borrow();
            let pos = (*p_readlines.borrow()).min(data.len());
            let remaining = &data[pos..];
            let lines: Vec<PyObjectRef> = remaining.split_inclusive(|&c| c == b'\n').map(|line| {
                if text { py_str(&decode_bytes(line, &enc_readlines)) } else { PyObjectRef::imm(PyObject::Bytes(line.to_vec())) }
            }).collect();
            *p_readlines.borrow_mut() = data.len();
            Ok(py_list(lines))
        }))));

        let b_iter = buf_rc.clone();
        let p_iter = pos_rc.clone();
        let enc_iter = encoding_owned.clone();
        type_dict.insert_str("__iter__", PyObjectRef::new(PyObject::Closure(Rc::new(move |self_args: &[PyObjectRef]| {
            let _ = (&b_iter, &p_iter, &enc_iter);
            Ok(self_args.first().cloned().unwrap_or_else(py_none))
        }))));
        let b_next = buf_rc.clone();
        let p_next = pos_rc.clone();
        let enc_next = encoding_owned.clone();
        type_dict.insert_str("__next__", PyObjectRef::new(PyObject::Closure(Rc::new(move |_: &[PyObjectRef]| {
            let data = b_next.borrow();
            let pos = (*p_next.borrow()).min(data.len());
            if pos >= data.len() { return Err(PyError::StopIteration); }
            let remaining = &data[pos..];
            let end = remaining.iter().position(|&c| c == b'\n').map(|i| i + 1).unwrap_or(remaining.len());
            let chunk = remaining[..end].to_vec();
            *p_next.borrow_mut() = pos + end;
            if text {
                Ok(py_str(&decode_bytes(&chunk, &enc_next)))
            } else {
                Ok(PyObjectRef::imm(PyObject::Bytes(chunk)))
            }
        }))));

        type_dict.insert_str("close", PyObjectRef::new(PyObject::Closure(Rc::new(move |_: &[PyObjectRef]| Ok(py_none())))));
    }

    type_dict.insert_str("__enter__", PyObjectRef::new(PyObject::BuiltinFunction {
        name: "__enter__".to_string(), func: |args: &[PyObjectRef]| Ok(args[0].clone()),
    }));
    let close_for_exit = type_dict.get_str("close").cloned();
    if let Some(close_fn) = close_for_exit {
        type_dict.insert_str("__exit__", PyObjectRef::new(PyObject::Closure(Rc::new(move |_: &[PyObjectRef]| {
            call_function(&close_fn, vec![])?;
            Ok(py_bool(false))
        }))));
    }

    Ok(PyObjectRef::new(PyObject::Instance {
        typ: PyObjectRef::new(PyObject::Type { name: "GzipFile".to_string(), dict: Box::new(str_map_to_typedict(type_dict)), bases: vec![], mro: vec![] }),
        dict: AttrMap::new(),
    }))
}

fn decode_bytes(bytes: &[u8], encoding: &str) -> String {
    match encoding.to_ascii_lowercase().as_str() {
        "latin-1" | "latin1" | "iso-8859-1" => bytes.iter().map(|&b| b as char).collect(),
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}

pub fn create_gzip_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! gz_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    // gzip header FLG bits (see RFC 1952)
    d.insert_str("FTEXT", py_int(1));
    d.insert_str("FHCRC", py_int(2));
    d.insert_str("FEXTRA", py_int(4));
    d.insert_str("FNAME", py_int(8));
    d.insert_str("FCOMMENT", py_int(16));

    gz_func!("open", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("open() takes at least 1 argument (filename)"));
        }
        let filename = args[0].borrow().str();
        let mut mode = "rb".to_string();
        let mut encoding = "utf-8".to_string();
        for a in &args[1..] {
            match &*a.borrow() {
                PyObject::Str(s) => mode = s.to_string(),
                PyObject::Dict(dct) => {
                    if let Ok(Some(v)) = dct.get(&py_str("mode")) { mode = v.str(); }
                    if let Ok(Some(v)) = dct.get(&py_str("encoding")) { encoding = v.str(); }
                }
                _ => {}
            }
        }
        let text = mode.contains('t');
        let binary_mode: String = mode.chars().filter(|&c| c != 't').collect();
        let binary_mode = if binary_mode.is_empty() || binary_mode == "r" || binary_mode == "w" || binary_mode == "a" {
            format!("{}b", binary_mode)
        } else {
            binary_mode
        };
        build_gzip_file(&filename, &binary_mode, 9, None, text, &encoding)
    });

    gz_func!("GzipFile", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("GzipFile() requires a filename"));
        }
        let filename = args[0].borrow().str();
        let mut mode = "rb".to_string();
        let mut compresslevel: u32 = 9;
        for a in &args[1..] {
            match &*a.borrow() {
                PyObject::Str(s) => mode = s.to_string(),
                PyObject::Int(i) => { if let Some(n) = i.to_i64() { compresslevel = n as u32; } }
                PyObject::Dict(dct) => {
                    if let Ok(Some(v)) = dct.get(&py_str("mode")) { mode = v.str(); }
                    if let Ok(Some(v)) = dct.get(&py_str("compresslevel")) {
                        if let Some(n) = v.as_i64() { compresslevel = n as u32; }
                    }
                }
                _ => {}
            }
        }
        build_gzip_file(&filename, &mode, compresslevel, None, false, "utf-8")
    });

    gz_func!("compress", |args| {
        if args.is_empty() { return Err(PyError::type_error("compress() takes at least 1 argument")); }
        let bytes = gzip_bytes_arg(&args[0])?;
        let (compresslevel, mtime) = gzip_parse_level_mtime(&args[1..]);
        let mut encoder = flate2::GzBuilder::new()
            .mtime(mtime.unwrap_or(0))
            .write(Vec::new(), flate2::Compression::new(compresslevel.min(9)));
        encoder.write_all(&bytes).map_err(|e| PyError::OsError(format!("{}", e)))?;
        let result = encoder.finish().map_err(|e| PyError::OsError(format!("{}", e)))?;
        Ok(PyObjectRef::new(PyObject::Bytes(result)))
    });

    gz_func!("decompress", |args| {
        if args.len() != 1 { return Err(PyError::type_error("decompress() takes exactly one argument")); }
        let bytes = gzip_bytes_arg(&args[0])?;
        let mut decoder = flate2::read::GzDecoder::new(&bytes[..]);
        let mut out = Vec::new();
        decoder.read_to_end(&mut out).map_err(|e| PyError::OsError(format!("gzip decompress error: {}", e)))?;
        Ok(PyObjectRef::new(PyObject::Bytes(out)))
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
        let mut inst_dict = AttrMap::new();
        inst_dict.insert_str("name", py_str(&_name));
        inst_dict.insert_str("getnames", PyObjectRef::new(PyObject::BuiltinFunction {
            name: "getnames".to_string(),
            func: |_args| Ok(py_list(vec![])),
        }));
        inst_dict.insert_str("extractall", PyObjectRef::new(PyObject::BuiltinFunction {
            name: "extractall".to_string(),
            func: |_args| Ok(py_none()),
        }));
        Ok(PyObjectRef::new(PyObject::Instance {
            typ: PyObjectRef::new(PyObject::Module {
                name: "tarfile.TarFile".to_string(),
                dict: Box::new(str_map_to_typedict(HashMap::new())),
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
            dict.insert_str("_path", path_val);
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
        path_type_dict.insert_str("parent", PyObjectRef::new(PyObject::Property(Box::new(PropertyData {
            getter: Some(getter),
            setter: None,
            deleter: None,
            doc: None,
        }))));
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
        path_type_dict.insert_str("name", PyObjectRef::new(PyObject::Property(Box::new(PropertyData {
            getter: Some(getter),
            setter: None,
            deleter: None,
            doc: None,
        }))));
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
        path_type_dict.insert_str("suffix", PyObjectRef::new(PyObject::Property(Box::new(PropertyData {
            getter: Some(getter),
            setter: None,
            deleter: None,
            doc: None,
        }))));
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
        path_type_dict.insert_str("stem", PyObjectRef::new(PyObject::Property(Box::new(PropertyData {
            getter: Some(getter),
            setter: None,
            deleter: None,
            doc: None,
        }))));
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
        let path_type = PATH_TYPE.with(|cell| {
            cell.borrow().clone()
        }).ok_or_else(|| PyError::runtime_error("Path type not initialized".to_string()))?;
        let mut instance_dict = AttrMap::new();
        instance_dict.insert_str("_path", py_str(&result));
        Ok(PyObjectRef::new(PyObject::Instance {
            typ: path_type,
            dict: instance_dict,
        }))
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

// Moved here from object.rs (was under a "---- zipfile module ----" banner
// in the monolithic object.rs — see the file-splitting refactor's memory
// entry for context).
// Helper: extract ZIP entry data from an Instance's dict
fn zipfile_get_entry(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let self_obj = &args[0];
    let name = args[1].borrow().str();
    let (entries, data) = match &*self_obj.borrow() {
        PyObject::Instance { dict, .. } => {
            let entries = dict.get("_entries").ok_or_else(|| PyError::runtime_error("ZipFile instance corrupted: missing _entries"))?.clone();
            let data = dict.get("_data").ok_or_else(|| PyError::runtime_error("ZipFile instance corrupted: missing _data"))?.clone();
            (entries, data)
        }
        _ => return Err(PyError::runtime_error("ZipFile method called on non-instance")),
    };

    let entries_list = match &*entries.borrow() {
        PyObject::List(items) => items.clone(),
        _ => return Err(PyError::runtime_error("ZipFile entries corrupted")),
    };

    let data_bytes = match &*data.borrow() {
        PyObject::Bytes(b) => b.clone(),
        _ => return Err(PyError::runtime_error("ZipFile data corrupted")),
    };

    for entry in &entries_list {
        let entry_borrow = entry.borrow();
        let entry_list = match &*entry_borrow {
            PyObject::List(items) => items,
            _ => continue,
        };
        if entry_list.len() < 5 { continue; }
        let entry_name = entry_list[0].borrow().str();
        if entry_name != name {
            continue;
        }
        let data_offset = match entry_list[1].as_i64() { Some(n) => n as usize, None => continue };
        let compressed_size = match entry_list[2].as_i64() { Some(n) => n as usize, None => continue };
        if data_offset + compressed_size > data_bytes.len() {
            return Err(PyError::runtime_error("ZipFile: data truncated in archive"));
        }
        let raw = data_bytes[data_offset..data_offset + compressed_size].to_vec();
        return Ok(PyObjectRef::new(PyObject::Bytes(raw)));
    }

    Err(PyError::key_error(format!("File not found in zip: '{}'", name)))
}

fn zipfile_namelist(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error("namelist() requires self"));
    }
    match &*args[0].borrow() {
        PyObject::Instance { dict, .. } => {
            if let Some(names) = dict.get("_names") {
                return Ok(names.clone());
            }
            Err(PyError::runtime_error("ZipFile instance corrupted: missing _names"))
        }
        _ => Err(PyError::runtime_error("namelist() called on non-instance")),
    }
}

fn zipfile_read(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Err(PyError::type_error("read() takes exactly one argument (name)"));
    }
    zipfile_get_entry(args)
}

fn zipfile_extract(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Err(PyError::type_error("extract() takes exactly one argument (name)"));
    }
    zipfile_get_entry(args)
}

fn zipfile_infolist(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let self_obj = &args[0];
    let entries = match &*self_obj.borrow() {
        PyObject::Instance { dict, .. } => {
            dict.get("_entries").ok_or_else(|| PyError::runtime_error("ZipFile instance corrupted"))?.clone()
        }
        _ => return Err(PyError::runtime_error("infolist() called on non-instance")),
    };

    let entries_list = match &*entries.borrow() {
        PyObject::List(items) => items.clone(),
        _ => return Err(PyError::runtime_error("ZipFile entries corrupted")),
    };

    let infos: Vec<PyObjectRef> = entries_list.iter().map(|entry| {
        let entry_borrow = entry.borrow();
        let entry_list = match &*entry_borrow {
            PyObject::List(items) => items,
            _ => return py_none(),
        };
        let mut info_dict = AttrMap::new();
        if entry_list.len() >= 1 {
            info_dict.insert("filename".to_string(), entry_list[0].clone());
        }
        if entry_list.len() >= 4 {
            info_dict.insert("file_size".to_string(), entry_list[3].clone());
        }
        if entry_list.len() >= 3 {
            info_dict.insert("compress_size".to_string(), entry_list[2].clone());
        }
        PyObjectRef::new(PyObject::Instance {
            typ: PyObjectRef::new(PyObject::Module {
                name: "zipfile.ZipInfo".to_string(),
                dict: Box::new(TypeDict::default()),
            }),
            dict: info_dict,
        })
    }).collect();

    Ok(py_list(infos))
}

pub fn zipfile_constructor(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 1 || args.len() > 2 {
        return Err(PyError::type_error("ZipFile() takes 1-2 arguments (filename, [mode])"));
    }
    let filename = args[0].borrow().str();
    let mode = if args.len() > 1 { args[1].borrow().str() } else { "r".to_string() };
    if mode != "r" {
        return Err(PyError::value_error("ZipFile only supports mode='r'"));
    }

    // Read entire file into memory
    let archive = match std::fs::read(&filename) {
        Ok(d) => d,
        Err(e) => return Err(PyError::runtime_error(format!("Cannot open zip file '{}': {}", filename, e))),
    };

    // Scan for local file headers (signature 0x04034b50)
    let archive_len = archive.len();
    let mut offset = 0usize;
    // entries stored as Vec of Python lists: [name, data_offset, compressed_size, uncompressed_size, compress_method]
    let mut names: Vec<PyObjectRef> = Vec::new();
    let mut entries: Vec<PyObjectRef> = Vec::new();

    loop {
        if offset + 30 > archive_len {
            break;
        }
        let sig = u32::from_le_bytes([
            archive[offset],
            archive[offset + 1],
            archive[offset + 2],
            archive[offset + 3],
        ]);
        if sig != 0x04034b50 {
            // Not a local file header — reached central directory or end
            break;
        }

        let compressed_size = u32::from_le_bytes([
            archive[offset + 18], archive[offset + 19],
            archive[offset + 20], archive[offset + 21],
        ]) as usize;
        let uncompressed_size = u32::from_le_bytes([
            archive[offset + 22], archive[offset + 23],
            archive[offset + 24], archive[offset + 25],
        ]) as usize;
        let filename_length = u16::from_le_bytes([archive[offset + 26], archive[offset + 27]]) as usize;
        let extra_field_length = u16::from_le_bytes([archive[offset + 28], archive[offset + 29]]) as usize;

        let name_start = offset + 30;
        let data_start = name_start + filename_length + extra_field_length;

        let name = if filename_length > 0 && name_start + filename_length <= archive_len {
            String::from_utf8_lossy(&archive[name_start..name_start + filename_length]).to_string()
        } else {
            String::new()
        };

        names.push(py_str(&name));
        entries.push(PyObjectRef::new(PyObject::List(vec![
            py_str(&name),
            py_int(data_start as i64),
            py_int(compressed_size as i64),
            py_int(uncompressed_size as i64),
            // compress_method stored separately in entries_meta if needed
        ])));

        offset = data_start + compressed_size;
    }

    let mut inst_dict = AttrMap::new();
    inst_dict.insert("filename".to_string(), py_str(&filename));
    inst_dict.insert("_data".to_string(), PyObjectRef::new(PyObject::Bytes(archive)));
    inst_dict.insert("_names".to_string(), py_list(names));
    inst_dict.insert("_entries".to_string(), py_list(entries));

    // Attach methods as BuiltinFunctions (will be wrapped as BuiltinMethod with self_obj)
    inst_dict.insert("namelist".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "namelist".to_string(),
        func: zipfile_namelist,
    }));
    inst_dict.insert("read".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "read".to_string(),
        func: zipfile_read,
    }));
    inst_dict.insert("extract".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "extract".to_string(),
        func: zipfile_extract,
    }));
    inst_dict.insert("infolist".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "infolist".to_string(),
        func: zipfile_infolist,
    }));

    Ok(PyObjectRef::new(PyObject::Instance {
        typ: PyObjectRef::new(PyObject::Module {
            name: "zipfile.ZipFile".to_string(),
            dict: Box::new(TypeDict::default()),
        }),
        dict: inst_dict,
    }))
}

pub fn create_zipfile_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    d.insert_str("ZipFile", PyObjectRef::new(PyObject::BuiltinFunction {
        name: "ZipFile".to_string(),
        func: zipfile_constructor,
    }));
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
        PyObject::Instance { dict, .. } => {
            match dict.get("_data") {
                Some(data) => Ok(data.clone()),
                None => Err(PyError::runtime_error("Shelf instance corrupted: missing _data")),
            }
        }
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
        return Err(PyError::type_error("get() takes at least 2 arguments (self, key)"));
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
        let pairs: Vec<PyObjectRef> = d.items().into_iter().map(|(k, v)| {
            PyObjectRef::new(PyObject::Tuple(vec![k, v]))
        }).collect();
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
        return Err(PyError::type_error("__contains__() takes at least 2 arguments (self, key)"));
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
        return Err(PyError::type_error("__getitem__() takes at least 2 arguments (self, key)"));
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
        return Err(PyError::type_error("__setitem__() takes at least 3 arguments (self, key, value)"));
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
        return Err(PyError::type_error("__delitem__() takes at least 2 arguments (self, key)"));
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
        return Err(PyError::type_error("open() takes at least 1 argument (filename)"));
    }
    let filename = args[0].str();

    // Internal data dict
    let data_dict = py_dict();

    // Instance dict with field and methods
    let mut inst_dict = AttrMap::new();
    inst_dict.insert("_data".to_string(), data_dict);
    inst_dict.insert("filename".to_string(), py_str(&filename));

    inst_dict.insert("close".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "close".to_string(), func: shelf_close }));
    inst_dict.insert("sync".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "sync".to_string(), func: shelf_sync }));
    inst_dict.insert("get".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "get".to_string(), func: shelf_get }));
    inst_dict.insert("keys".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "keys".to_string(), func: shelf_keys }));
    inst_dict.insert("values".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "values".to_string(), func: shelf_values }));
    inst_dict.insert("items".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "items".to_string(), func: shelf_items }));

    // Type dict with dunder methods (used by py_getitem/py_setitem dispatch)
    let mut type_dict = HashMap::new();
    type_dict.insert("__getitem__".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "__getitem__".to_string(), func: shelf_getitem }));
    type_dict.insert("__setitem__".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "__setitem__".to_string(), func: shelf_setitem }));
    type_dict.insert("__delitem__".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "__delitem__".to_string(), func: shelf_delitem }));
    type_dict.insert("__len__".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "__len__".to_string(), func: shelf_len }));
    type_dict.insert("__contains__".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "__contains__".to_string(), func: shelf_contains }));
    type_dict.insert("__repr__".to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: "__repr__".to_string(), func: shelf_repr }));

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

pub fn create_shelve_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    d.insert_str("open", PyObjectRef::new(PyObject::BuiltinFunction { name: "open".to_string(), func: shelf_open }));
    d.insert_str("Shelf", py_str("Shelf"));
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

