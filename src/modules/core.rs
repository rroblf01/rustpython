use crate::object::*;
use std::collections::HashMap;
use num_traits::Signed;
pub fn create_builtins() -> HashMap<String, PyObjectRef> {
    let mut builtins = HashMap::new();
    builtins.insert("None".to_string(), py_none());
    builtins.insert("True".to_string(), py_bool(true));
    builtins.insert("False".to_string(), py_bool(false));
    builtins.insert("Ellipsis".to_string(), PyObjectRef::imm(PyObject::Str("...".to_string())));

    macro_rules! add_func {
        ($name:expr, $func:expr) => {
            builtins.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
                name: $name.to_string(),
                func: $func,
            }));
        };
    }

    add_func!("print", builtin_print);
    add_func!("len", builtin_len);
    add_func!("range", builtin_range);
    add_func!("type", builtin_type_of);
    add_func!("int", builtin_int);
    add_func!("float", builtin_float);
    add_func!("str", builtin_str);
    add_func!("bool", builtin_bool);
    add_func!("list", builtin_list);
    add_func!("tuple", builtin_tuple);
    add_func!("dict", builtin_dict);
    add_func!("set", builtin_set);
    add_func!("abs", builtin_abs);
    add_func!("hasattr", builtin_hasattr);
    add_func!("getattr", builtin_getattr);
    add_func!("setattr", builtin_setattr);
    add_func!("delattr", builtin_delattr);
    add_func!("ord", builtin_ord);
    add_func!("chr", builtin_chr);
    add_func!("hex", builtin_hex);
    add_func!("oct", builtin_oct);
    add_func!("bin", builtin_bin);
    add_func!("ascii", builtin_ascii);
    add_func!("memoryview", builtin_memoryview);
    add_func!("input", builtin_input);
    add_func!("exit", builtin_exit);
    add_func!("repr", builtin_repr);
    add_func!("sorted", builtin_sorted);
    add_func!("enumerate", builtin_enumerate);
    add_func!("iter", builtin_iter);
    add_func!("next", builtin_next);
    add_func!("sum", builtin_sum);
    add_func!("max", builtin_max);
    add_func!("min", builtin_min);
    add_func!("id", builtin_id);
    add_func!("vars", builtin_vars);
    add_func!("isinstance", builtin_isinstance);
    add_func!("open", builtin_open);
    add_func!("any", builtin_any);
    add_func!("all", builtin_all);
    add_func!("callable", builtin_callable);
    add_func!("breakpoint", builtin_breakpoint);
    add_func!("pow", builtin_pow);
    add_func!("reversed", builtin_reversed);
    add_func!("issubclass", builtin_issubclass);
    add_func!("help", builtin_help);
    add_func!("eval", builtin_eval);
    add_func!("exec", builtin_exec);
    add_func!("compile", builtin_compile);
    add_func!("super", builtin_super);
    add_func!("map", builtin_map);
    add_func!("filter", builtin_filter);
    add_func!("zip", builtin_zip);
    add_func!("property", builtin_property);
    add_func!("staticmethod", builtin_staticmethod);
    add_func!("classmethod", builtin_classmethod);
    add_func!("bytes", builtin_bytes);
    add_func!("bytearray", builtin_bytearray);
    add_func!("frozenset", builtin_frozenset);
    add_func!("format", builtin_format);
    add_func!("object", builtin_object);
    add_func!("hash", builtin_hash);
    add_func!("slice", builtin_slice);
    add_func!("divmod", builtin_divmod);
    add_func!("round", builtin_round);
    add_func!("dir", builtin_dir);
    add_func!("globals", builtin_globals);
    add_func!("locals", builtin_locals);

    macro_rules! add_exc_type {
        ($name:expr, $func:expr) => {
            builtins.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
                name: $name.to_string(),
                func: $func,
            }));
        };
    }

    add_exc_type!("BaseException", builtin_make_exception_baseexception);
    add_exc_type!("Exception", builtin_make_exception_exception);
    add_exc_type!("TypeError", builtin_make_exception_typeerror);
    add_exc_type!("ValueError", builtin_make_exception_valueerror);
    add_exc_type!("ZeroDivisionError", builtin_make_exception_zerodivisionerror);
    add_exc_type!("NameError", builtin_make_exception_nameerror);
    add_exc_type!("AttributeError", builtin_make_exception_attributeerror);
    add_exc_type!("IndexError", builtin_make_exception_indexerror);
    add_exc_type!("KeyError", builtin_make_exception_keyerror);
    add_exc_type!("RuntimeError", builtin_make_exception_runtimeerror);
    add_exc_type!("StopIteration", builtin_make_exception_stopiteration);
    add_exc_type!("AssertionError", builtin_make_exception_assertionerror);
    add_exc_type!("OSError", builtin_make_exception_oserror);
    add_exc_type!("ImportError", builtin_make_exception_importerror);
    add_exc_type!("LookupError", builtin_make_exception_lookuperror);
    add_exc_type!("ArithmeticError", builtin_make_exception_arithmeticerror);
    add_exc_type!("FloatingPointError", builtin_make_exception_floatingpointerror);
    add_exc_type!("OverflowError", builtin_make_exception_overflowerror);
    add_exc_type!("EnvironmentError", builtin_make_exception_environmenterror);
    add_exc_type!("IOError", builtin_make_exception_ioerror);
    add_exc_type!("FileNotFoundError", builtin_make_exception_filenotfounderror);
    add_exc_type!("PermissionError", builtin_make_exception_permissionerror);
    add_exc_type!("NotImplementedError", builtin_make_exception_notimplementederror);
    add_exc_type!("RecursionError", builtin_make_exception_recursionerror);
    add_exc_type!("KeyboardInterrupt", builtin_make_exception_keyboardinterrupt);
    add_exc_type!("GeneratorExit", builtin_make_exception_generatorexit);
    add_exc_type!("SystemExit", builtin_make_exception_systemexit);
    add_exc_type!("ModuleNotFoundError", builtin_make_exception_modulenotfounderror);
    add_exc_type!("StopAsyncIteration", builtin_make_exception_stopasynciteration);
    add_exc_type!("EOFError", builtin_make_exception_eoferror);
    add_exc_type!("ConnectionError", builtin_make_exception_connectionerror);
    add_exc_type!("BrokenPipeError", builtin_make_exception_brokenpipeerror);
    add_exc_type!("ConnectionRefusedError", builtin_make_exception_connectionrefusederror);
    add_exc_type!("BlockingIOError", builtin_make_exception_blockingioerror);
    add_exc_type!("ChildProcessError", builtin_make_exception_childprocesserror);
    add_exc_type!("InterruptedError", builtin_make_exception_interruptederror);
    add_exc_type!("TimeoutError", builtin_make_exception_timeouterror);
    add_exc_type!("UnicodeDecodeError", builtin_make_exception_unicodedecodeerror);
    add_exc_type!("UnicodeEncodeError", builtin_make_exception_unicodeencodeerror);

    let math_module = PyObjectRef::new(PyObject::Module {
        name: "math".to_string(),
        dict: create_math_dict(),
    });
    builtins.insert("math".to_string(), math_module.clone());

    builtins
}

pub fn create_math_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! math_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }
    math_func!("sqrt", |args| {
        if args.len() != 1 { return Err(PyError::type_error("sqrt() takes exactly one argument")); }
        let v = args[0].borrow();
        match &*v { PyObject::Int(i) => Ok(py_float(i.to_f64().unwrap_or(0.0).sqrt())), PyObject::Float(f) => Ok(py_float(f.sqrt())), _ => Err(PyError::type_error("sqrt() argument must be a number")) }
    });
    math_func!("sin", |args| {
        if args.len() != 1 { return Err(PyError::type_error("sin() takes exactly one argument")); }
        let v = args[0].borrow();
        match &*v { PyObject::Int(i) => Ok(py_float(i.to_f64().unwrap_or(0.0).sin())), PyObject::Float(f) => Ok(py_float(f.sin())), _ => Err(PyError::type_error("sin() argument must be a number")) }
    });
    math_func!("cos", |args| {
        if args.len() != 1 { return Err(PyError::type_error("cos() takes exactly one argument")); }
        let v = args[0].borrow();
        match &*v { PyObject::Int(i) => Ok(py_float(i.to_f64().unwrap_or(0.0).cos())), PyObject::Float(f) => Ok(py_float(f.cos())), _ => Err(PyError::type_error("cos() argument must be a number")) }
    });
    math_func!("tan", |args| {
        if args.len() != 1 { return Err(PyError::type_error("tan() takes exactly one argument")); }
        let v = args[0].borrow();
        match &*v { PyObject::Int(i) => Ok(py_float(i.to_f64().unwrap_or(0.0).tan())), PyObject::Float(f) => Ok(py_float(f.tan())), _ => Err(PyError::type_error("tan() argument must be a number")) }
    });
    math_func!("floor", |args| {
        if args.len() != 1 { return Err(PyError::type_error("floor() takes exactly one argument")); }
        let v = args[0].borrow();
        match &*v { PyObject::Int(i) => Ok(py_int(i.clone())), PyObject::Float(f) => Ok(py_int(f.floor() as i64)), _ => Err(PyError::type_error("floor() argument must be a number")) }
    });
    math_func!("ceil", |args| {
        if args.len() != 1 { return Err(PyError::type_error("ceil() takes exactly one argument")); }
        let v = args[0].borrow();
        match &*v { PyObject::Int(i) => Ok(py_int(i.clone())), PyObject::Float(f) => Ok(py_int(f.ceil() as i64)), _ => Err(PyError::type_error("ceil() argument must be a number")) }
    });
    math_func!("exp", |args| {
        if args.len() != 1 { return Err(PyError::type_error("exp() takes exactly one argument")); }
        let v = args[0].borrow();
        match &*v { PyObject::Int(i) => Ok(py_float(i.to_f64().unwrap_or(0.0).exp())), PyObject::Float(f) => Ok(py_float(f.exp())), _ => Err(PyError::type_error("exp() argument must be a number")) }
    });
    math_func!("pow", |args| {
        if args.len() != 2 { return Err(PyError::type_error("pow() takes exactly two arguments")); }
        let a = args[0].borrow();
        let b = args[1].borrow();
        match (&*a, &*b) {
            (PyObject::Int(i), PyObject::Int(j)) => Ok(py_float(i.to_f64().unwrap_or(0.0).powf(j.to_f64().unwrap_or(0.0)))),
            (PyObject::Int(i), PyObject::Float(f)) => Ok(py_float(i.to_f64().unwrap_or(0.0).powf(*f))),
            (PyObject::Float(f), PyObject::Int(i)) => Ok(py_float(f.powf(i.to_f64().unwrap_or(0.0)))),
            (PyObject::Float(a), PyObject::Float(b)) => Ok(py_float(a.powf(*b))),
            _ => Err(PyError::type_error("pow() argument must be a number")),
        }
    });
    math_func!("log", |args| {
        if args.len() < 1 || args.len() > 2 { return Err(PyError::type_error("log() takes one or two arguments")); }
        let v = args[0].borrow();
        let x = match &*v { PyObject::Int(i) => i.to_f64().unwrap_or(0.0), PyObject::Float(f) => *f, _ => return Err(PyError::type_error("log() argument must be a number")) };
        let base = if args.len() == 2 {
            let b = args[1].borrow();
            match &*b { PyObject::Int(i) => i.to_f64().unwrap_or(0.0), PyObject::Float(f) => *f, _ => return Err(PyError::type_error("log() base must be a number")) }
        } else {
            std::f64::consts::E
        };
        Ok(py_float(x.log(base)))
    });
    math_func!("abs", |args| {
        if args.len() != 1 { return Err(PyError::type_error("abs() takes exactly one argument")); }
        let v = args[0].borrow();
        match &*v { PyObject::Int(i) => Ok(py_float(i.to_f64().unwrap_or(0.0).abs())), PyObject::Float(f) => Ok(py_float(f.abs())), _ => Err(PyError::type_error("abs() argument must be a number")) }
    });
    d.insert("pi".to_string(), py_float(std::f64::consts::PI));
    d.insert("e".to_string(), py_float(std::f64::consts::E));
    d
}

pub fn create_sys_dict(argv: Vec<String>) -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! sys_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }
    sys_func!("exit", |args| {
        let code = if args.len() > 0 {
            match &*args[0].borrow() {
                PyObject::Int(i) => i.to_i64().unwrap_or(0) as i32,
                _ => 1,
            }
        } else { 0 };
        Err(PyError::SystemExit(code))
    });
    d.insert("argv".to_string(), py_list(argv.into_iter().map(|s| py_str(&s)).collect()));
    d.insert("path".to_string(), py_list(vec![]));
    d.insert("modules".to_string(), py_dict());
    d.insert("version".to_string(), py_str("3.12.0 (RustPython 0.1.0)"));
    d.insert("stdin".to_string(), PyObjectRef::new(PyObject::File {
        file: std::rc::Rc::new(std::cell::RefCell::new(std::fs::File::open("/dev/stdin").unwrap_or_else(|_| {
            // Fallback: create a temporary file
            std::fs::File::create("/dev/null").unwrap()
        }))),
    }));
    d.insert("stdout".to_string(), PyObjectRef::new(PyObject::File {
        file: std::rc::Rc::new(std::cell::RefCell::new(std::fs::File::create("/dev/stdout").unwrap_or_else(|_| {
            std::fs::File::create("/dev/null").unwrap()
        }))),
    }));
    d.insert("stderr".to_string(), PyObjectRef::new(PyObject::File {
        file: std::rc::Rc::new(std::cell::RefCell::new(std::fs::File::create("/dev/stderr").unwrap_or_else(|_| {
            std::fs::File::create("/dev/null").unwrap()
        }))),
    }));
    d.insert("platform".to_string(), py_str(std::env::consts::OS));
    d.insert("implementation".to_string(), py_str("RustPython"));
    d.insert("byteorder".to_string(), py_str(if cfg!(target_endian = "little") { "little" } else { "big" }));
    d.insert("maxsize".to_string(), py_int(i64::MAX));
    d.insert("maxunicode".to_string(), py_int(1114111));
    d.insert("api_version".to_string(), py_int(1013));
    d.insert("executable".to_string(), py_str(&std::env::current_exe().map(|p| p.to_string_lossy().to_string()).unwrap_or_default()));
    d.insert("prefix".to_string(), py_str("/usr"));
    d.insert("exec_prefix".to_string(), py_str("/usr"));
    d.insert("winver".to_string(), py_str("3.12"));
    d
}

pub fn create_os_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! os_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }
    os_func!("listdir", |args| {
        let path = if args.len() > 0 { args[0].str() } else { ".".to_string() };
        match std::fs::read_dir(&path) {
            Ok(entries) => {
                let names: Vec<PyObjectRef> = entries.filter_map(|e| e.ok()).map(|e| py_str(&e.file_name().to_string_lossy())).collect();
                Ok(py_list(names))
            }
            Err(e) => Err(PyError::OsError(format!("{}", e))),
        }
    });
    os_func!("mkdir", |args| {
        if args.is_empty() { return Err(PyError::type_error("mkdir() takes at least 1 argument")); }
        match std::fs::create_dir(&args[0].str()) {
            Ok(()) => Ok(py_none()),
            Err(e) => Err(PyError::OsError(format!("{}", e))),
        }
    });
    os_func!("remove", |args| {
        if args.is_empty() { return Err(PyError::type_error("remove() takes at least 1 argument")); }
        match std::fs::remove_file(&args[0].str()) {
            Ok(()) => Ok(py_none()),
            Err(e) => Err(PyError::OsError(format!("{}", e))),
        }
    });
    os_func!("rename", |args| {
        if args.len() < 2 { return Err(PyError::type_error("rename() takes 2 arguments")); }
        match std::fs::rename(&args[0].str(), &args[1].str()) {
            Ok(()) => Ok(py_none()),
            Err(e) => Err(PyError::OsError(format!("{}", e))),
        }
    });

    os_func!("system", |args| {
        if args.is_empty() { return Err(PyError::type_error("system() takes at least 1 argument")); }
        let cmd = args[0].str();
        match std::process::Command::new("/bin/sh").arg("-c").arg(&cmd).status() {
            Ok(status) => Ok(py_int(status.code().unwrap_or(0) as i64)),
            Err(e) => Err(PyError::OsError(format!("{}", e))),
        }
    });

    os_func!("chdir", |args| {
        if args.is_empty() { return Err(PyError::type_error("chdir() takes at least 1 argument")); }
        match std::env::set_current_dir(&args[0].str()) {
            Ok(()) => Ok(py_none()),
            Err(e) => Err(PyError::OsError(format!("{}", e))),
        }
    });

    os_func!("getcwd", |_| {
        match std::env::current_dir() {
            Ok(p) => Ok(py_str(&p.to_string_lossy())),
            Err(e) => Err(PyError::OsError(format!("{}", e))),
        }
    });

    os_func!("getenv", |args| {
        if args.is_empty() { return Ok(py_none()); }
        let key = args[0].str();
        match std::env::var(&key) {
            Ok(val) => Ok(py_str(&val)),
            Err(_) => {
                if args.len() > 1 { Ok(args[1].clone()) }
                else { Ok(py_none()) }
            }
        }
    });

    os_func!("putenv", |args| {
        if args.len() < 2 { return Err(PyError::type_error("putenv() takes exactly 2 arguments")); }
        std::env::set_var(args[0].str(), args[1].str());
        Ok(py_none())
    });

    os_func!("unsetenv", |args| {
        if args.is_empty() { return Err(PyError::type_error("unsetenv() takes at least 1 argument")); }
        std::env::remove_var(args[0].str());
        Ok(py_none())
    });

    // File descriptor operations
    os_func!("open", |args| {
        if args.len() < 2 { return Err(PyError::type_error("open() requires at least 2 arguments")); }
        let path = args[0].str();
        let flags = args[1].as_i64().unwrap_or(0) as i32;
        let mut opts = std::fs::OpenOptions::new();
        // O_RDONLY=0, O_WRONLY=1, O_RDWR=2 — check access mode
        let access_mode = flags & 3;
        if access_mode == 0 { opts.read(true); }     // O_RDONLY
        if access_mode == 1 { opts.write(true); }    // O_WRONLY
        if access_mode == 2 { opts.write(true); opts.read(true); } // O_RDWR
        if flags & 64 != 0 { opts.create(true); }       // O_CREAT = 64
        if flags & 512 != 0 { opts.truncate(true); }    // O_TRUNC = 512
        if flags & 1024 != 0 { opts.append(true); }     // O_APPEND = 1024
        match opts.open(&path) {
            Ok(file) => {
                use std::os::unix::io::IntoRawFd;
                Ok(py_int(file.into_raw_fd() as i64))
            }
            Err(e) => Err(PyError::OsError(format!("{}", e))),
        }
    });
    os_func!("read", |args| {
        if args.len() < 2 { return Err(PyError::type_error("read() requires at least 2 arguments")); }
        let fd = args[0].as_i64().unwrap_or(-1) as i32;
        let n = args[1].as_i64().unwrap_or(0) as usize;
        use std::os::unix::io::FromRawFd;
        let mut buf = vec![0u8; n];
        let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
        use std::io::Read;
        match file.read(&mut buf) {
            Ok(count) => {
                buf.truncate(count);
                std::mem::forget(file); // Don't close the fd
                Ok(PyObjectRef::new(PyObject::Bytes(buf)))
            }
            Err(e) => {
                std::mem::forget(file);
                Err(PyError::OsError(format!("{}", e)))
            }
        }
    });
    os_func!("write", |args| {
        if args.len() < 2 { return Err(PyError::type_error("write() requires at least 2 arguments")); }
        let fd = args[0].as_i64().unwrap_or(-1) as i32;
        let data = match &*args[1].borrow() {
            PyObject::Bytes(b) => b.clone(),
            PyObject::Str(s) => s.as_bytes().to_vec(),
            _ => return Err(PyError::type_error("write() argument 2 must be bytes or str")),
        };
        use std::os::unix::io::FromRawFd;
        let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
        use std::io::Write;
        match file.write(&data) {
            Ok(count) => {
                std::mem::forget(file);
                Ok(py_int(count as i64))
            }
            Err(e) => {
                std::mem::forget(file);
                Err(PyError::OsError(format!("{}", e)))
            }
        }
    });
    os_func!("close", |args| {
        if args.is_empty() { return Err(PyError::type_error("close() requires at least 1 argument")); }
        let fd = args[0].as_i64().unwrap_or(-1) as i32;
        use std::os::unix::io::FromRawFd;
        let file = unsafe { std::fs::File::from_raw_fd(fd) };
        drop(file); // Closes the fd
        Ok(py_none())
    });

    // OS flags for open()
    d.insert("O_RDONLY".to_string(), py_int(0));
    d.insert("O_WRONLY".to_string(), py_int(1));
    d.insert("O_RDWR".to_string(), py_int(2));
    d.insert("O_CREAT".to_string(), py_int(64));
    d.insert("O_TRUNC".to_string(), py_int(512));
    d.insert("O_APPEND".to_string(), py_int(1024));

    // environ dict
    let mut environ_dict = HashMap::new();
    for (key, val) in std::env::vars() {
        environ_dict.insert(key, py_str(&val));
    }
    d.insert("environ".to_string(), create_module("environ", environ_dict));

    // os.path sub-module
    let mut path_dict = HashMap::new();
    macro_rules! path_func {
        ($name:expr, $func:expr) => {
            path_dict.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }
    path_func!("join", |args| {
        let parts: Vec<String> = args.iter().map(|a| a.str()).collect();
        if parts.is_empty() { return Ok(py_str("")); }
        let result = parts.join("/");
        Ok(py_str(&result))
    });
    path_func!("dirname", |args| {
        if args.is_empty() { return Err(PyError::type_error("dirname() takes at least 1 argument")); }
        let path = args[0].str();
        let result = match path.rfind('/') {
            Some(i) => if i == 0 { "/".to_string() } else { path[..i].to_string() },
            None => ".".to_string(),
        };
        Ok(py_str(&result))
    });
    path_func!("basename", |args| {
        if args.is_empty() { return Err(PyError::type_error("basename() takes at least 1 argument")); }
        let path = args[0].str();
        let result = match path.rfind('/') {
            Some(i) => path[i+1..].to_string(),
            None => path,
        };
        Ok(py_str(&result))
    });
    path_func!("split", |args| {
        if args.is_empty() { return Err(PyError::type_error("split() takes at least 1 argument")); }
        let path = args[0].str();
        let (head, tail) = match path.rfind('/') {
            Some(i) => {
                let h = if i == 0 { "/".to_string() } else { path[..i].to_string() };
                let t = path[i+1..].to_string();
                (h, t)
            }
            None => (".".to_string(), path.clone()),
        };
        Ok(py_list(vec![py_str(&head), py_str(&tail)]))
    });
    path_func!("splitext", |args| {
        if args.is_empty() { return Err(PyError::type_error("splitext() takes at least 1 argument")); }
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
    path_func!("exists", |args| {
        if args.is_empty() { return Err(PyError::type_error("exists() takes at least 1 argument")); }
        Ok(py_bool(std::path::Path::new(&args[0].str()).exists()))
    });
    path_func!("isfile", |args| {
        if args.is_empty() { return Err(PyError::type_error("isfile() takes at least 1 argument")); }
        Ok(py_bool(std::path::Path::new(&args[0].str()).is_file()))
    });
    path_func!("isdir", |args| {
        if args.is_empty() { return Err(PyError::type_error("isdir() takes at least 1 argument")); }
        Ok(py_bool(std::path::Path::new(&args[0].str()).is_dir()))
    });
    d.insert("path".to_string(), create_module("path", path_dict));
    d
}

pub fn create_operator_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! op_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    op_func!("add", |args| {
        if args.len() < 2 { return Err(PyError::type_error("operator.add requires 2 arguments")); }
        py_add(&args[0], &args[1])
    });
    op_func!("sub", |args| {
        if args.len() < 2 { return Err(PyError::type_error("operator.sub requires 2 arguments")); }
        py_sub(&args[0], &args[1])
    });
    op_func!("mul", |args| {
        if args.len() < 2 { return Err(PyError::type_error("operator.mul requires 2 arguments")); }
        py_mul(&args[0], &args[1])
    });
    op_func!("truediv", |args| {
        if args.len() < 2 { return Err(PyError::type_error("operator.truediv requires 2 arguments")); }
        py_div(&args[0], &args[1])
    });
    op_func!("floordiv", |args| {
        if args.len() < 2 { return Err(PyError::type_error("operator.floordiv requires 2 arguments")); }
        py_floor_div(&args[0], &args[1])
    });
    op_func!("mod", |args| {
        if args.len() < 2 { return Err(PyError::type_error("operator.mod requires 2 arguments")); }
        py_mod(&args[0], &args[1])
    });
    op_func!("pow", |args| {
        if args.len() < 2 { return Err(PyError::type_error("operator.pow requires 2 arguments")); }
        py_pow(&args[0], &args[1])
    });
    op_func!("lt", |args| {
        if args.len() < 2 { return Err(PyError::type_error("operator.lt requires 2 arguments")); }
        py_compare(&args[0], &args[1], 0)
    });
    op_func!("le", |args| {
        if args.len() < 2 { return Err(PyError::type_error("operator.le requires 2 arguments")); }
        py_compare(&args[0], &args[1], 1)
    });
    op_func!("eq", |args| {
        if args.len() < 2 { return Err(PyError::type_error("operator.eq requires 2 arguments")); }
        py_compare(&args[0], &args[1], 2)
    });
    op_func!("ne", |args| {
        if args.len() < 2 { return Err(PyError::type_error("operator.ne requires 2 arguments")); }
        py_compare(&args[0], &args[1], 5)
    });
    op_func!("ge", |args| {
        if args.len() < 2 { return Err(PyError::type_error("operator.ge requires 2 arguments")); }
        py_compare(&args[0], &args[1], 3)
    });
    op_func!("gt", |args| {
        if args.len() < 2 { return Err(PyError::type_error("operator.gt requires 2 arguments")); }
        py_compare(&args[0], &args[1], 4)
    });
    op_func!("and_", |args| {
        if args.len() < 2 { return Err(PyError::type_error("operator.and_ requires 2 arguments")); }
        py_bit_and(&args[0], &args[1])
    });
    op_func!("or_", |args| {
        if args.len() < 2 { return Err(PyError::type_error("operator.or_ requires 2 arguments")); }
        py_bit_or(&args[0], &args[1])
    });
    op_func!("xor", |args| {
        if args.len() < 2 { return Err(PyError::type_error("operator.xor requires 2 arguments")); }
        py_bit_xor(&args[0], &args[1])
    });
    op_func!("not_", |args| {
        if args.is_empty() { return Err(PyError::type_error("operator.not_ requires 1 argument")); }
        Ok(py_not(&args[0]))
    });
    op_func!("getitem", |args| {
        if args.len() < 2 { return Err(PyError::type_error("operator.getitem requires 2 arguments")); }
        py_getitem(&args[0], &args[1])
    });
    op_func!("setitem", |args| {
        if args.len() < 3 { return Err(PyError::type_error("operator.setitem requires 3 arguments")); }
        py_setitem(&args[0], &args[1], args[2].clone())?;
        Ok(py_none())
    });
    op_func!("contains", |args| {
        if args.len() < 2 { return Err(PyError::type_error("operator.contains requires 2 arguments")); }
        py_contains(&args[0], &args[1])
    });
    op_func!("index", |args| {
        if args.is_empty() { return Err(PyError::type_error("operator.index requires 1 argument")); }
        to_index(&args[0]).map(|i| py_int(i))
    });
    op_func!("truth", |args| {
        if args.is_empty() { return Err(PyError::type_error("operator.truth requires 1 argument")); }
        Ok(py_bool(args[0].truthy()))
    });
    op_func!("neg", |args| {
        if args.is_empty() { return Err(PyError::type_error("operator.neg requires 1 argument")); }
        py_neg(&args[0])
    });
    op_func!("pos", |args| {
        if args.is_empty() { return Err(PyError::type_error("operator.pos requires 1 argument")); }
        Ok(args[0].clone())
    });
    op_func!("abs", |args| {
        if args.is_empty() { return Err(PyError::type_error("operator.abs requires 1 argument")); }
        if let Some(i) = args[0].as_i64() { return Ok(py_int(i.abs())); }
        if let Some(f) = args[0].as_f64() { return Ok(py_float(f.abs())); }
        let obj = args[0].borrow();
        match &*obj {
            PyObject::Int(n) => Ok(py_int(n.clone().abs())),
            PyObject::Float(n) => Ok(py_float(n.abs())),
            _ => Err(PyError::type_error(format!("bad operand type for abs(): '{}'", obj.type_name()))),
        }
    });
    op_func!("inv", |args| {
        if args.is_empty() { return Err(PyError::type_error("operator.inv requires 1 argument")); }
        if let Some(i) = args[0].as_i64() { return Ok(py_int(!i)); }
        let obj = args[0].borrow();
        if let PyObject::Int(n) = &*obj { Ok(py_int(!n.clone())) }
        else { Err(PyError::type_error(format!("bad operand type for inv(): '{}'", obj.type_name()))) }
    });
    op_func!("lshift", |args| {
        if args.len() < 2 { return Err(PyError::type_error("operator.lshift requires 2 arguments")); }
        py_lshift(&args[0], &args[1])
    });
    op_func!("rshift", |args| {
        if args.len() < 2 { return Err(PyError::type_error("operator.rshift requires 2 arguments")); }
        py_rshift(&args[0], &args[1])
    });
    op_func!("length_hint", |args| {
        if args.is_empty() { return Err(PyError::type_error("operator.length_hint requires 1 argument")); }
        builtin_len(args)
    });
    // __iadd__ etc. — just wrap the binop
    op_func!("__add__", |args| { if args.len() < 2 { return Err(PyError::type_error("__add__ requires 2 arguments")); } py_add(&args[0], &args[1]) });
    op_func!("__sub__", |args| { if args.len() < 2 { return Err(PyError::type_error("__sub__ requires 2 arguments")); } py_sub(&args[0], &args[1]) });
    op_func!("__mul__", |args| { if args.len() < 2 { return Err(PyError::type_error("__mul__ requires 2 arguments")); } py_mul(&args[0], &args[1]) });
    op_func!("__truediv__", |args| { if args.len() < 2 { return Err(PyError::type_error("__truediv__ requires 2 arguments")); } py_div(&args[0], &args[1]) });
    op_func!("__floordiv__", |args| { if args.len() < 2 { return Err(PyError::type_error("__floordiv__ requires 2 arguments")); } py_floor_div(&args[0], &args[1]) });
    op_func!("__mod__", |args| { if args.len() < 2 { return Err(PyError::type_error("__mod__ requires 2 arguments")); } py_mod(&args[0], &args[1]) });
    op_func!("__pow__", |args| { if args.len() < 2 { return Err(PyError::type_error("__pow__ requires 2 arguments")); } py_pow(&args[0], &args[1]) });
    op_func!("__and__", |args| { if args.len() < 2 { return Err(PyError::type_error("__and__ requires 2 arguments")); } py_bit_and(&args[0], &args[1]) });
    op_func!("__or__", |args| { if args.len() < 2 { return Err(PyError::type_error("__or__ requires 2 arguments")); } py_bit_or(&args[0], &args[1]) });
    op_func!("__xor__", |args| { if args.len() < 2 { return Err(PyError::type_error("__xor__ requires 2 arguments")); } py_bit_xor(&args[0], &args[1]) });
    op_func!("__lshift__", |args| { if args.len() < 2 { return Err(PyError::type_error("__lshift__ requires 2 arguments")); } py_lshift(&args[0], &args[1]) });
    op_func!("__rshift__", |args| { if args.len() < 2 { return Err(PyError::type_error("__rshift__ requires 2 arguments")); } py_rshift(&args[0], &args[1]) });
    op_func!("__getitem__", |args| { if args.len() < 2 { return Err(PyError::type_error("__getitem__ requires 2 arguments")); } py_getitem(&args[0], &args[1]) });
    op_func!("__setitem__", |args| { if args.len() < 3 { return Err(PyError::type_error("__setitem__ requires 3 arguments")); } py_setitem(&args[0], &args[1], args[2].clone())?; Ok(py_none()) });

    // itemgetter factory
    d.insert("itemgetter".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "itemgetter".to_string(),
        func: |args| {
            if args.is_empty() { return Err(PyError::type_error("itemgetter requires at least 1 argument")); }
            let items = args.to_vec();
            // Return a callable that does getitem on its argument
            let getter = PyObjectRef::new(PyObject::Closure(Rc::new(move |get_args| {
                if get_args.is_empty() { return Err(PyError::type_error("itemgetter called with no arguments")); }
                let obj = &get_args[0];
                if items.len() == 1 {
                    py_getitem(obj, &items[0])
                } else {
                    let mut results = Vec::new();
                    for item in &items {
                        results.push(py_getitem(obj, item)?);
                    }
                    Ok(PyObjectRef::imm(PyObject::Tuple(results)))
                }
            })));
            Ok(getter)
        },
    }));

    // attrgetter factory
    d.insert("attrgetter".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "attrgetter".to_string(),
        func: |args| {
            if args.is_empty() { return Err(PyError::type_error("attrgetter requires at least 1 argument")); }
            let attrs: Vec<String> = args.iter().map(|a| a.str()).collect();
            let getter = PyObjectRef::new(PyObject::Closure(Rc::new(move |get_args| {
                if get_args.is_empty() { return Err(PyError::type_error("attrgetter called with no arguments")); }
                if attrs.len() == 1 {
                    get_args[0].borrow().get_attribute(&attrs[0])
                } else {
                    let mut results = Vec::new();
                    for attr in &attrs {
                        results.push(get_args[0].borrow().get_attribute(attr)?);
                    }
                    Ok(PyObjectRef::imm(PyObject::Tuple(results)))
                }
            })));
            Ok(getter)
        },
    }));

    d
}

use std::rc::Rc;
use std::cell::RefCell;
use num_traits::ToPrimitive;