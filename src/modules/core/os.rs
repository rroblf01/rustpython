use crate::object::*;
use std::collections::HashMap;
use std::sync::atomic::AtomicI64;
use num_traits::{Signed, ToPrimitive};
use std::rc::Rc;

mod helpers;
mod path;

pub(crate) use helpers::dup_std_fd;
pub use helpers::os_kill_builtin;
pub use path::create_os_path_dict;
pub(crate) use helpers::{close_fd, fstat_result, lseek_fd, os_path_arg, read_fd, stat_dev_ino, stat_to_dict, write_fd};

pub fn create_os_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! os_func {
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
    d.insert_str("linesep", py_str(if cfg!(windows) { "\r\n" } else { "\n" }));
    d.insert_str(
        "defpath",
        py_str(if cfg!(windows) { "." } else { ":/bin:/usr/bin" }),
    );
    d.insert_str(
        "devnull",
        py_str(if cfg!(windows) { "nul" } else { "/dev/null" }),
    );
    // `os.F_OK`/`R_OK`/`W_OK`/`X_OK` + `os.access()` — missing entirely.
    // Matches the real POSIX bitmask values (`F_OK=0`, `X_OK=1`, `W_OK=2`,
    // `R_OK=4`) so `mode` values combine the same way real code expects
    // (`os.access(path, os.R_OK | os.W_OK)`).
    d.insert_str("F_OK", py_int(0));
    d.insert_str("X_OK", py_int(1));
    d.insert_str("W_OK", py_int(2));
    d.insert_str("R_OK", py_int(4));
    // `os.SEEK_SET`/`SEEK_CUR`/`SEEK_END` — the whence constants for
    // `os.lseek`/`file.seek` (real POSIX values 0/1/2).
    d.insert_str("SEEK_SET", py_int(0));
    d.insert_str("SEEK_CUR", py_int(1));
    d.insert_str("SEEK_END", py_int(2));
    os_func!("lseek", |args| {
        if args.len() < 3 {
            return Err(PyError::type_error("lseek() requires 3 arguments"));
        }
        let fd = args[0]
            .as_i64()
            .ok_or_else(|| PyError::type_error("an integer is required (got type fd)"))?
            as i32;
        let offset = args[1]
            .as_i64()
            .ok_or_else(|| PyError::type_error("an integer is required"))?;
        let whence = args[2]
            .as_i64()
            .ok_or_else(|| PyError::type_error("an integer is required"))?
            as i32;
        match lseek_fd(fd, offset, whence) {
            Ok(pos) => Ok(py_int(pos)),
            Err(e) => Err(PyError::os_error_from_io(&e)),
        }
    });
    os_func!("access", |args| {
        if args.is_empty() {
            return Err(PyError::type_error(
                "access() missing required argument: 'path'",
            ));
        }
        let path = args[0].str();
        // Best-effort: this interpreter has no real per-bit POSIX
        // permission-checking machinery (setuid/gid, ACLs, etc.) — F_OK
        // (existence) is always answerable exactly; R_OK/W_OK/X_OK fall
        // back to the same "path exists" answer, which is correct often
        // enough for typical test usage (checking a file it just created
        // is readable/writable) without claiming full POSIX fidelity.
        Ok(py_bool(std::fs::metadata(&path).is_ok()))
    });
    os_func!("fspath", |args| {
        if args.is_empty() {
            return Err(PyError::type_error(
                "fspath() missing required argument: 'path'",
            ));
        }
        let obj = args[0].clone();
        match &*obj.borrow() {
            PyObject::Str(_) | PyObject::Bytes(_) => return Ok(obj.clone()),
            _ => {}
        }
        if let Ok(f) = obj.borrow().get_attribute("__fspath__") {
            return call_bound_method(f, obj.clone(), vec![]);
        }
        Err(PyError::type_error(format!(
            "expected str, bytes or os.PathLike object, not {}",
            obj.borrow().type_name()
        )))
    });
    os_func!("fsencode", |args| {
        if args.is_empty() {
            return Err(PyError::type_error(
                "fsencode() missing required argument: 'filename'",
            ));
        }
        // Must resolve the PEP 519 `__fspath__` protocol (a path-like
        // wrapper, e.g. `pathlib.Path` or a test-only `FakePath`) — was
        // stringifying the wrapper object directly (its repr), producing
        // completely wrong bytes for anything but a plain `str`/`bytes`
        // argument. Confirmed via `test_dbm.py::test_whichdb`, which feeds
        // `os_helper.FakePath`-wrapped paths through `dbm.whichdb` ->
        // `os.fsencode`.
        let s = crate::object::path_arg_to_string(&args[0]);
        Ok(PyObjectRef::imm(PyObject::Bytes(s.into_bytes())))
    });
    os_func!("fsdecode", |args| {
        if args.is_empty() {
            return Err(PyError::type_error(
                "fsdecode() missing required argument: 'filename'",
            ));
        }
        let s = crate::object::path_arg_to_string(&args[0]);
        Ok(py_str(&s))
    });
    os_func!("listdir", |args| {
        let path = if args.len() > 0 {
            args[0].str()
        } else {
            ".".to_string()
        };
        match std::fs::read_dir(&path) {
            Ok(entries) => {
                let names: Vec<PyObjectRef> = entries
                    .filter_map(|e| e.ok())
                    .map(|e| py_str(&e.file_name().to_string_lossy()))
                    .collect();
                Ok(py_list(names))
            }
            Err(e) => Err(PyError::os_error_from_io(&e)),
        }
    });
    os_func!("mkdir", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("mkdir() takes at least 1 argument"));
        }
        match std::fs::create_dir(&args[0].str()) {
            Ok(()) => Ok(py_none()),
            Err(e) => Err(PyError::os_error_from_io(&e)),
        }
    });
    os_func!("remove", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("remove() takes at least 1 argument"));
        }
        let path = crate::object::path_arg_to_string(&args[0]);
        std::fs::remove_file(&path).map_err(|e| PyError::os_error_from_io(&e))?;
        Ok(py_none())
    });

    // os.unlink = os.remove (POSIX alias)
    os_func!("unlink", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("unlink() takes at least 1 argument"));
        }
        let path = crate::object::path_arg_to_string(&args[0]);
        std::fs::remove_file(&path).map_err(|e| PyError::os_error_from_io(&e))?;
        Ok(py_none())
    });

    os_func!("rename", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("rename() takes 2 arguments"));
        }
        match std::fs::rename(
            &crate::object::path_arg_to_string(&args[0]),
            &crate::object::path_arg_to_string(&args[1]),
        ) {
            Ok(()) => Ok(py_none()),
            Err(e) => Err(PyError::os_error_from_io(&e)),
        }
    });

    os_func!("system", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("system() takes at least 1 argument"));
        }
        let cmd = args[0].str();
        match std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(&cmd)
            .status()
        {
            Ok(status) => Ok(py_int(status.code().unwrap_or(0) as i64)),
            Err(e) => Err(PyError::os_error_from_io(&e)),
        }
    });

    os_func!("chdir", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("chdir() takes at least 1 argument"));
        }
        match std::env::set_current_dir(&args[0].str()) {
            Ok(()) => Ok(py_none()),
            Err(e) => Err(PyError::os_error_from_io(&e)),
        }
    });

    os_func!("getcwd", |_| {
        match std::env::current_dir() {
            Ok(p) => Ok(py_str(&p.to_string_lossy())),
            Err(e) => Err(PyError::os_error_from_io(&e)),
        }
    });

    // `os.isatty(fd)` — was missing entirely (`AttributeError`), breaking
    // `test__colorize.py`'s `setUpModule`/tests, which `unittest.mock.patch`
    // this out anyway (so the real return value never matters for THAT
    // file — it just needs the attribute to exist to be patchable). Still
    // implemented for real using `std::io::IsTerminal` (stable stdlib,
    // no new dependency) for the standard fds 0/1/2; any other fd number
    // conservatively reports `False` (this project has no generic
    // fd-to-terminal check without pulling in `libc`).
    os_func!("isatty", |args| {
        let fd = args.first().and_then(|a| a.as_i64()).unwrap_or(-1);
        use std::io::IsTerminal;
        let result = match fd {
            0 => std::io::stdin().is_terminal(),
            1 => std::io::stdout().is_terminal(),
            2 => std::io::stderr().is_terminal(),
            _ => false,
        };
        Ok(py_bool(result))
    });

    os_func!("getenv", |args| {
        if args.is_empty() {
            return Ok(py_none());
        }
        let key = args[0].str();
        match std::env::var(&key) {
            Ok(val) => Ok(py_str(&val)),
            Err(_) => {
                if args.len() > 1 {
                    Ok(args[1].clone())
                } else {
                    Ok(py_none())
                }
            }
        }
    });

    os_func!("putenv", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("putenv() takes exactly 2 arguments"));
        }
        std::env::set_var(args[0].str(), args[1].str());
        Ok(py_none())
    });

    os_func!("unsetenv", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("unsetenv() takes at least 1 argument"));
        }
        std::env::remove_var(args[0].str());
        Ok(py_none())
    });

    // File descriptor operations
    os_func!("open", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("open() requires at least 2 arguments"));
        }
        let path = args[0].str();
        let flags = args[1].as_i64().unwrap_or(0) as i32;
        let mut opts = std::fs::OpenOptions::new();
        // O_RDONLY=0, O_WRONLY=1, O_RDWR=2 — check access mode
        let access_mode = flags & 3;
        if access_mode == 0 {
            opts.read(true);
        } // O_RDONLY
        if access_mode == 1 {
            opts.write(true);
        } // O_WRONLY
        if access_mode == 2 {
            opts.write(true);
            opts.read(true);
        } // O_RDWR
        if flags & 64 != 0 {
            // O_CREAT = 64
            if flags & 128 != 0 {
                // O_EXCL = 128
                opts.create_new(true);
            } else {
                opts.create(true);
            }
        }
        if flags & 512 != 0 {
            opts.truncate(true);
        } // O_TRUNC = 512
        if flags & 1024 != 0 {
            opts.append(true);
        } // O_APPEND = 1024
        match opts.open(&path) {
            Ok(file) => {
                use std::os::unix::io::IntoRawFd;
                Ok(py_int(file.into_raw_fd() as i64))
            }
            Err(e) => Err(PyError::os_error_from_io(&e)),
        }
    });
    os_func!("read", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("read() requires at least 2 arguments"));
        }
        let fd = args[0].as_i64().unwrap_or(-1) as i32;
        let n = args[1].as_i64().unwrap_or(0) as usize;
        let mut buf = vec![0u8; n];
        match read_fd(fd, &mut buf) {
            Ok(count) => {
                buf.truncate(count);
                Ok(PyObjectRef::new(PyObject::Bytes(buf)))
            }
            Err(e) => Err(PyError::os_error_from_io(&e)),
        }
    });
    os_func!("write", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("write() requires at least 2 arguments"));
        }
        let fd = args[0].as_i64().unwrap_or(-1) as i32;
        let data = match &*args[1].borrow() {
            PyObject::Bytes(b) => b.clone(),
            PyObject::Str(s) => s.as_bytes().to_vec(),
            _ => {
                return Err(PyError::type_error(
                    "write() argument 2 must be bytes or str",
                ))
            }
        };
        match write_fd(fd, &data) {
            Ok(count) => Ok(py_int(count as i64)),
            Err(e) => Err(PyError::os_error_from_io(&e)),
        }
    });
    os_func!("close", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("close() requires at least 1 argument"));
        }
        let fd = args[0].as_i64().unwrap_or(-1) as i32;
        close_fd(fd);
        Ok(py_none())
    });

    // os.fdopen(fd, mode='r') -> file object from fd
    os_func!("fdopen", |args| {
        if args.is_empty() {
            return Err(PyError::type_error(
                "fdopen() missing required argument 'fd'",
            ));
        }
        let fd = args[0]
            .as_i64()
            .ok_or_else(|| PyError::type_error("fd must be an integer"))? as i32;
        let (pos_args, kwargs) = match args.last() {
            Some(last) if matches!(&*last.borrow(), PyObject::Dict(_)) => (&args[..args.len()-1], Some(last)),
            _ => (args, None),
        };
        let mode = if pos_args.len() > 1 {
            pos_args[1].str()
        } else if let Some(kw) = kwargs {
            if let PyObject::Dict(d) = &*kw.borrow() {
                d.get(&py_str("mode")).ok().flatten().map(|v| v.str()).unwrap_or_else(|| "r".to_string())
            } else { "r".to_string() }
        } else {
            "r".to_string()
        };
        let mut encoding: Option<String> = None;
        let mut errors: Option<String> = None;
        if let Some(kw) = kwargs {
            if let PyObject::Dict(d) = &*kw.borrow() {
                if let Some(v) = d.get(&py_str("encoding")).ok().flatten() {
                    if !matches!(&*v.borrow(), PyObject::None) {
                        encoding = Some(v.str());
                    }
                }
                if let Some(v) = d.get(&py_str("errors")).ok().flatten() {
                    if !matches!(&*v.borrow(), PyObject::None) {
                        errors = Some(v.str());
                    }
                }
            }
        }
        if encoding.is_none() && pos_args.len() > 3 {
            let v = &pos_args[3];
            if !matches!(&*v.borrow(), PyObject::None) {
                let s = v.str();
                if s.parse::<i64>().is_err() {
                    encoding = Some(s);
                }
            }
        }
        if errors.is_none() && pos_args.len() > 4 {
            let v = &pos_args[4];
            if !matches!(&*v.borrow(), PyObject::None) {
                errors = Some(v.str());
            }
        }
        let binary = mode.contains('b');
        if binary {
            if encoding.is_some() {
                return Err(PyError::value_error("binary mode doesn't take an encoding argument"));
            }
            if errors.is_some() {
                return Err(PyError::value_error("binary mode doesn't take an errors argument"));
            }
        }
        use std::os::unix::io::FromRawFd;
        let file = unsafe { std::fs::File::from_raw_fd(fd) };
        Ok(PyObjectRef::new(PyObject::File {
            file: std::rc::Rc::new(std::cell::RefCell::new(file)),
            name: format!("<fdopen>"),
            binary,
            pending: std::rc::Rc::new(std::cell::RefCell::new(Vec::new())),
            closed: false,
        }))
    });

    // os.urandom(size) -> random bytes from OS
    os_func!("urandom", |args| {
        if args.is_empty() {
            return Err(PyError::type_error(
                "urandom() requires at least 1 argument",
            ));
        }
        let n = args[0]
            .as_i64()
            .ok_or_else(|| PyError::type_error("argument must be an integer"))?;
        if n <= 0 {
            return Ok(PyObjectRef::imm(PyObject::Bytes(Vec::new())));
        }
        let mut buf = vec![0u8; n as usize];
        if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
            use std::io::Read;
            let _ = f.read_exact(&mut buf);
        }
        Ok(PyObjectRef::imm(PyObject::Bytes(buf)))
    });

    // OS flags for open()
    d.insert_str("O_RDONLY", py_int(0));
    d.insert_str("O_WRONLY", py_int(1));
    d.insert_str("O_RDWR", py_int(2));
    d.insert_str("O_CREAT", py_int(64));
    d.insert_str("O_EXCL", py_int(128));
    d.insert_str("O_TRUNC", py_int(512));
    d.insert_str("O_APPEND", py_int(1024));

    // environ dict — use a proper PyDict instead of Module so methods like
    // .setdefault(), .get(), .keys(), 'x in environ', etc. all work (Django req.)
    let mut environ_pydict = PyDict::new();
    for (key, val) in std::env::vars() {
        environ_pydict.set(py_str(&key), py_str(&val)).ok();
    }
    d.insert_str(
        "environ",
        PyObjectRef::new(PyObject::Dict(Box::new(environ_pydict))),
    );

    // --- os.getpid() ---
    os_func!("getpid", |_| { Ok(py_int(std::process::id() as i64)) });

    // `os.kill(pid, sig)` — was missing entirely (`AttributeError`), breaking
    // any test that uses the common "send myself a signal" pattern to
    // exercise a registered `signal.signal()` handler (real trigger:
    // `test_threadsignals.py`'s `acquire_retries_on_intr`). Only meaningful
    // for OUR OWN pid in this single-process interpreter (there is no real
    // multi-process signal delivery to hook into) — actually invoking the
    // handler needs a live `&mut VirtualMachine`, so the real work happens
    // via `vm.rs`'s own special case for this exact function pointer (see
    // `crate::modules::invoke_signal_handler_impl`); this is the
    // `with_vm_mut`-based fallback for any path that reaches it without
    // going through that special case.
    os_func!("kill", os_kill_builtin);

    // --- os.getppid() ---
    os_func!("getppid", |_| {
        // Parse /proc/self/stat for parent PID
        match std::fs::read_to_string("/proc/self/stat") {
            Ok(stat) => {
                if let Some(idx) = stat.rfind(')') {
                    let fields: Vec<&str> = stat[idx + 1..].split_whitespace().collect();
                    if fields.len() > 1 {
                        if let Ok(ppid) = fields[1].parse::<i64>() {
                            return Ok(py_int(ppid));
                        }
                    }
                }
                Err(PyError::OsError(
                    "failed to parse /proc/self/stat".to_string(),
                ))
            }
            Err(e) => Err(PyError::os_error_from_io(&e)),
        }
    });

    // --- os.cpu_count() ---
    os_func!("cpu_count", |_| {
        match std::thread::available_parallelism() {
            Ok(n) => Ok(py_int(n.get() as i64)),
            Err(_) => Ok(py_none()),
        }
    });

    // --- os.getloadavg() ---
    os_func!("getloadavg", |_| {
        match std::fs::read_to_string("/proc/loadavg") {
            Ok(data) => {
                let parts: Vec<&str> = data.split_whitespace().collect();
                if parts.len() >= 3 {
                    let load1: f64 = parts[0].parse().unwrap_or(0.0);
                    let load5: f64 = parts[1].parse().unwrap_or(0.0);
                    let load15: f64 = parts[2].parse().unwrap_or(0.0);
                    Ok(py_tuple(vec![
                        py_float(load1),
                        py_float(load5),
                        py_float(load15),
                    ]))
                } else {
                    Ok(py_tuple(vec![py_float(0.0), py_float(0.0), py_float(0.0)]))
                }
            }
            Err(_) => Ok(py_tuple(vec![py_float(0.0), py_float(0.0), py_float(0.0)])),
        }
    });

    // --- os.stat(path, *, dir_fd=None, follow_symlinks=True) ---
    // Accepts an integer file descriptor (like CPython): a bool is
    // additionally warned about ("bool is used as a file descriptor") and
    // then treated as fd 0/1.
    os_func!("stat", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("stat() takes at least 1 argument"));
        }
        if let Some(fd) = args[0].as_i64() {
            if matches!(args[0], PyObjectRef::SmallBool(_)) {
                crate::modules::dev::warnings_emit(
                    "bool is used as a file descriptor",
                    "RuntimeWarning",
                );
            }
            return fstat_result(fd);
        }
        let path = os_path_arg(&args[0])?;
        match std::fs::metadata(&path) {
            Ok(meta) => Ok(create_module("stat_result", stat_to_dict(&meta))),
            Err(e) => Err(PyError::os_error_from_io(&e)),
        }
    });

    // --- os.fstat(fd) ---
    os_func!("fstat", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("fstat() takes at least 1 argument"));
        }
        match args[0].as_i64() {
            Some(fd) => fstat_result(fd),
            None => Err(PyError::type_error(
                "fstat() argument must be an integer file descriptor",
            )),
        }
    });

    // --- os.lstat(path) ---
    os_func!("lstat", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("lstat() takes at least 1 argument"));
        }
        let path = os_path_arg(&args[0])?;
        match std::fs::symlink_metadata(&path) {
            Ok(meta) => Ok(create_module("stat_result", stat_to_dict(&meta))),
            Err(e) => Err(PyError::os_error_from_io(&e)),
        }
    });

    // --- stat_result module with field index constants ---
    {
        let mut sr = HashMap::new();
        sr.insert_str("ST_MODE", py_int(0));
        sr.insert_str("ST_INO", py_int(1));
        sr.insert_str("ST_DEV", py_int(2));
        sr.insert_str("ST_NLINK", py_int(3));
        sr.insert_str("ST_UID", py_int(4));
        sr.insert_str("ST_GID", py_int(5));
        sr.insert_str("ST_SIZE", py_int(6));
        sr.insert_str("ST_ATIME", py_int(7));
        sr.insert_str("ST_MTIME", py_int(8));
        sr.insert_str("ST_CTIME", py_int(9));
        sr.insert_str("n_fields", py_int(10));
        sr.insert_str("n_sequence_fields", py_int(10));
        sr.insert_str(
            "__doc__",
            py_str("stat_result: stat results as a module with named field indices"),
        );
        d.insert_str("stat_result", create_module("stat_result", sr));
    }

    // --- os.chmod(path, mode) ---
    os_func!("chmod", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("chmod() takes at least 2 arguments"));
        }
        let path = crate::object::path_arg_to_string(&args[0]);
        let mode = args[1].as_i64().unwrap_or(0) as u32;
        use std::os::unix::fs::PermissionsExt;
        match std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)) {
            Ok(()) => Ok(py_none()),
            Err(e) => Err(PyError::os_error_from_io(&e)),
        }
    });

    // --- os.utime(path, times=None) ---
    os_func!("utime", |args| {
        if args.is_empty() {
            return Err(PyError::type_error(
                "utime() missing required argument: 'path'",
            ));
        }
        let path = crate::object::path_arg_to_string(&args[0]);
        if !std::path::Path::new(&path).exists() {
            return Err(PyError::file_not_found_error(format!(
                "No such file or directory: '{}'",
                path
            )));
        }
        // Parse optional positional `times` and keyword `ns`/`times`.
        let mut times_val: Option<PyObjectRef> = None;
        let mut ns_val: Option<PyObjectRef> = None;
        if args.len() >= 2 && !matches!(&*args[1].borrow(), PyObject::Dict(_)) {
            times_val = Some(args[1].clone());
            // `None` positional means "now"
            if matches!(&*times_val.as_ref().unwrap().borrow(), PyObject::None) {
                times_val = None;
            }
        }
        for arg in args.iter() {
            if let PyObject::Dict(d) = &*arg.borrow() {
                if let Ok(Some(v)) = d.get(&py_str("ns")) {
                    if !matches!(&*v.borrow(), PyObject::None) {
                        ns_val = Some(v.clone());
                    } else {
                        ns_val = None;
                    }
                }
                if let Ok(Some(v)) = d.get(&py_str("times")) {
                    if matches!(&*v.borrow(), PyObject::None) {
                        times_val = None;
                    } else {
                        times_val = Some(v.clone());
                    }
                }
            }
        }
        // Apply times via filetime crate.
        if let Some(ns) = ns_val {
            // ns is a 2-tuple of nanoseconds (int)
            let (atime_ns, mtime_ns) = {
                let b = ns.borrow();
                match &*b {
                    PyObject::Tuple(items) | PyObject::List(items) => {
                        if items.len() != 2 {
                            return Err(PyError::type_error("ns must be a 2-tuple"));
                        }
                        let a = items[0]
                            .as_i64()
                            .ok_or_else(|| PyError::type_error("ns values must be integers"))?;
                        let m = items[1]
                            .as_i64()
                            .ok_or_else(|| PyError::type_error("ns values must be integers"))?;
                        (a, m)
                    }
                    _ => return Err(PyError::type_error("ns must be a 2-tuple")),
                }
            };
            let atime = filetime::FileTime::from_unix_time(atime_ns / 1_000_000_000, (atime_ns % 1_000_000_000) as u32);
            let mtime = filetime::FileTime::from_unix_time(mtime_ns / 1_000_000_000, (mtime_ns % 1_000_000_000) as u32);
            filetime::set_file_times(&path, atime, mtime)
                .map_err(|e| PyError::os_error_from_io(&e))?;
        } else if let Some(tv) = times_val {
            let (atime_s, mtime_s) = {
                let b = tv.borrow();
                match &*b {
                    PyObject::Tuple(items) | PyObject::List(items) => {
                        if items.len() != 2 {
                            return Err(PyError::type_error("times must be a 2-tuple"));
                        }
                        let a = items[0]
                            .as_f64()
                            .ok_or_else(|| PyError::type_error("times values must be numbers"))?;
                        let m = items[1]
                            .as_f64()
                            .ok_or_else(|| PyError::type_error("times values must be numbers"))?;
                        (a, m)
                    }
                    _ => return Err(PyError::type_error("times must be a 2-tuple")),
                }
            };
            let to_ft = |secs: f64| {
                let s = secs.trunc() as i64;
                let n = ((secs - s as f64) * 1e9) as u32;
                filetime::FileTime::from_unix_time(s, n)
            };
            let atime = to_ft(atime_s);
            let mtime = to_ft(mtime_s);
            filetime::set_file_times(&path, atime, mtime)
                .map_err(|e| PyError::os_error_from_io(&e))?;
        } else {
            // No times/ns -> set to now.
            let now = filetime::FileTime::now();
            filetime::set_file_times(&path, now, now)
                .map_err(|e| PyError::os_error_from_io(&e))?;
        }
        Ok(py_none())
    });

    // --- os.chown(path, uid, gid) ---
    os_func!("chown", |args| {
        if args.len() < 3 {
            return Err(PyError::type_error("chown() takes at least 3 arguments"));
        }
        let path = args[0].str();
        let uid = args[1].as_i64().unwrap_or(-1);
        let gid = args[2].as_i64().unwrap_or(-1);
        use std::os::unix::fs::chown;
        match chown(
            &path,
            if uid == -1 { None } else { Some(uid as u32) },
            if gid == -1 { None } else { Some(gid as u32) },
        ) {
            Ok(()) => Ok(py_none()),
            Err(e) => Err(PyError::os_error_from_io(&e)),
        }
    });

    // --- os.link(src, dst) ---
    os_func!("link", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("link() takes at least 2 arguments"));
        }
        match std::fs::hard_link(&args[0].str(), &args[1].str()) {
            Ok(()) => Ok(py_none()),
            Err(e) => Err(PyError::os_error_from_io(&e)),
        }
    });

    // --- os.symlink(src, dst) ---
    os_func!("symlink", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("symlink() takes at least 2 arguments"));
        }
        use std::os::unix::fs::symlink;
        match symlink(&args[0].str(), &args[1].str()) {
            Ok(()) => Ok(py_none()),
            Err(e) => Err(PyError::os_error_from_io(&e)),
        }
    });

    // --- os.readlink(path) ---
    os_func!("readlink", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("readlink() takes at least 1 argument"));
        }
        match std::fs::read_link(&args[0].str()) {
            Ok(p) => Ok(py_str(&p.to_string_lossy())),
            Err(e) => Err(PyError::os_error_from_io(&e)),
        }
    });

    // --- os.makedirs(path) ---
    os_func!("makedirs", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("makedirs() takes at least 1 argument"));
        }
        let path = args[0].str();
        match std::fs::create_dir_all(&path) {
            Ok(()) => Ok(py_none()),
            Err(e) => Err(PyError::os_error_from_io(&e)),
        }
    });

    // --- os.rmdir(path) ---
    os_func!("rmdir", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("rmdir() takes at least 1 argument"));
        }
        match std::fs::remove_dir(&args[0].str()) {
            Ok(()) => Ok(py_none()),
            Err(e) => Err(PyError::os_error_from_io(&e)),
        }
    });

    // --- os.walk(top): directory tree walker (returns list of tuples) ---
    os_func!("walk", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("walk() takes at least 1 argument"));
        }
        let top = args[0].str();
        let mut results = Vec::new();
        let mut dirs_to_visit = vec![top];
        while let Some(dir) = dirs_to_visit.pop() {
            match std::fs::read_dir(&dir) {
                Ok(entries) => {
                    let mut dirname_strs: Vec<String> = Vec::new();
                    let mut filename_strs: Vec<String> = Vec::new();
                    for entry in entries.flatten() {
                        let name = entry.file_name().to_string_lossy().to_string();
                        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
                        if name == "." || name == ".." {
                            continue;
                        }
                        if is_dir {
                            dirname_strs.push(name);
                        } else {
                            filename_strs.push(name);
                        }
                    }
                    dirname_strs.sort();
                    filename_strs.sort();
                    let dirnames: Vec<PyObjectRef> =
                        dirname_strs.iter().map(|s| py_str(s)).collect();
                    let filenames: Vec<PyObjectRef> =
                        filename_strs.iter().map(|s| py_str(s)).collect();
                    results.push(py_tuple(vec![
                        py_str(&dir),
                        py_list(dirnames),
                        py_list(filenames),
                    ]));
                    // Push subdirs in reverse order for DFS alphabetical traversal
                    for dirname in dirname_strs.iter().rev() {
                        let sub = if dir.ends_with('/') {
                            format!("{}{}", dir, dirname)
                        } else {
                            format!("{}/{}", dir, dirname)
                        };
                        dirs_to_visit.push(sub);
                    }
                }
                Err(_) => { /* skip unreadable directories */ }
            }
        }
        Ok(PyObjectRef::new(PyObject::List(results)))
    });

    // --- File mode constants (from <sys/stat.h>) ---
    d.insert_str("S_IFMT", py_int(0o170000));
    d.insert_str("S_IFSOCK", py_int(0o140000));
    d.insert_str("S_IFLNK", py_int(0o120000));
    d.insert_str("S_IFREG", py_int(0o100000));
    d.insert_str("S_IFBLK", py_int(0o060000));
    d.insert_str("S_IFDIR", py_int(0o040000));
    d.insert_str("S_IFCHR", py_int(0o020000));
    d.insert_str("S_IFIFO", py_int(0o010000));
    d.insert_str("S_ISUID", py_int(0o4000));
    d.insert_str("S_ISGID", py_int(0o2000));
    d.insert_str("S_ISVTX", py_int(0o1000));
    d.insert_str("S_IRWXU", py_int(0o700));
    d.insert_str("S_IRUSR", py_int(0o400));
    d.insert_str("S_IWUSR", py_int(0o200));
    d.insert_str("S_IXUSR", py_int(0o100));
    d.insert_str("S_IRWXG", py_int(0o070));
    d.insert_str("S_IRGRP", py_int(0o040));
    d.insert_str("S_IWGRP", py_int(0o020));
    d.insert_str("S_IXGRP", py_int(0o010));
    d.insert_str("S_IRWXO", py_int(0o007));
    d.insert_str("S_IROTH", py_int(0o004));
    d.insert_str("S_IWOTH", py_int(0o002));
    d.insert_str("S_IXOTH", py_int(0o001));

    // OS constants needed by stdlib code
    d.insert_str("name", py_str("posix"));
    d.insert_str("sep", py_str("/"));
    d.insert_str("linesep", py_str("\n"));
    d.insert_str("pathsep", py_str(":"));

    // `os.supports_dir_fd`/`supports_follow_symlinks`/`supports_effective_ids`/
    // `supports_fd` — real CPython exposes frozensets of the os functions
    // honoring each keyword. Ours honor NONE of them, so expose empty
    // frozensets: tests guard `{os.open, os.stat} <= os.supports_dir_fd`
    // (test_glob.py) and skip the dir_fd/symlinks path when the subset check
    // fails, falling back to the plain path — which is exactly what this
    // interpreter supports. `supports_bytes_environ` is a plain bool.
    let empty_frozen: PyObjectRef = crate::object::builtin_frozenset(&[])
        .unwrap_or_else(|_| PyObjectRef::imm(PyObject::FrozenSet(crate::object::PySet::new())));
    for name in [
        "supports_dir_fd",
        "supports_effective_ids",
        "supports_fd",
        "supports_follow_symlinks",
    ] {
        d.insert(name.to_string(), empty_frozen.clone());
    }
    d.insert_str("supports_bytes_environ", py_bool(true));

    // os.path sub-module will be wired as a proper submodule in vm.rs
    // The path attribute is set there (not inline) to allow proper os.path import
    d
}
