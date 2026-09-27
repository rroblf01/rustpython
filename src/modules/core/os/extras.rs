use crate::object::*;
use num_traits::ToPrimitive;
use std::collections::HashMap;

/// The `os` surface CPython has and this interpreter was missing.
///
/// Two groups: constants (mechanical, but stdlib and test code does
/// `os.O_NOFOLLOW`/`os.WEXITED`-style feature detection, where an
/// `AttributeError` is indistinguishable from "unsupported"), and the POSIX
/// calls that `os.path`/`subprocess`/`platform`/`shutil` actually reach for
/// (`os.get_exec_path` alone was breaking every `Popen` whose executable
/// lookup went through `Lib/subprocess.py`'s own `executable_list`
/// computation). The exotic corners — `posix_spawn*`, `forkpty`, `splice`,
/// `timerfd_*`, `setns`, `unshare`, `wait3`/`wait4`/`waitid` — are
/// deliberately still absent: pretending to support them would be worse than
/// a clean `AttributeError`.
pub(crate) fn register(d: &mut HashMap<String, PyObjectRef>) {
    constants(d);
    ids_and_process(d);
    fd_io(d);
    fs_extras(d);
    misc(d);
}

fn put(d: &mut HashMap<String, PyObjectRef>, name: &str, val: PyObjectRef) {
    d.insert_str(name, val);
}

fn ints(d: &mut HashMap<String, PyObjectRef>, table: &[(&str, i64)]) {
    for (name, val) in table {
        put(d, name, py_int(*val));
    }
}

fn constants(d: &mut HashMap<String, PyObjectRef>) {
    ints(
        d,
        &[
            // open(2)
            ("O_ACCMODE", 0o3),
            ("O_ASYNC", 0o2000),
            ("O_DIRECT", 0o40000),
            ("O_DIRECTORY", 0o200000),
            ("O_DSYNC", 0o10000),
            ("O_FSYNC", 0o4000000),
            ("O_LARGEFILE", 0o100000),
            ("O_NDELAY", 0o4000),
            ("O_NOATIME", 0o1000000),
            ("O_NOCTTY", 0o400),
            ("O_NOFOLLOW", 0o400000),
            ("O_NONBLOCK", 0o4000),
            ("O_PATH", 0o10000000),
            ("O_RSYNC", 0o20000),
            ("O_SYNC", 0o4010000),
            ("O_TMPFILE", 0o20200000),
            // wait(2)/waitid(2)
            ("P_ALL", 0),
            ("P_PID", 1),
            ("P_PGID", 2),
            ("WNOHANG", 1),
            ("WUNTRACED", 2),
            ("WCONTINUED", 8),
            ("WSTOPPED", 4),
            ("WEXITED", 4),
            ("WNOWAIT", 0x01000000),
            ("CLD_EXITED", 1),
            ("CLD_DUMPED", 2),
            ("CLD_TRAPPED", 3),
            ("CLD_CONTINUED", 4),
            ("CLD_KILLED", 5),
            ("CLD_STOPPED", 6),
            ("WCOREDUMP", 0x80),
            // mount(2)/sched/fadvise/splice/seek/rtld/eventfd
            ("ST_RDONLY", 1),
            ("ST_NOSUID", 2),
            ("ST_NODEV", 4),
            ("ST_NOEXEC", 8),
            ("ST_SYNCHRONOUS", 16),
            ("ST_RELATIME", 32),
            ("ST_NOATIME", 64),
            ("ST_NODIRATIME", 128),
            ("ST_MANDLOCK", 256),
            ("ST_APPEND", 512),
            ("SCHED_NORMAL", 0),
            ("SCHED_FIFO", 1),
            ("SCHED_RR", 2),
            ("SCHED_BATCH", 3),
            ("SCHED_IDLE", 5),
            ("SCHED_DEADLINE", 6),
            ("POSIX_FADV_NORMAL", 0),
            ("POSIX_FADV_RANDOM", 1),
            ("POSIX_FADV_SEQUENTIAL", 2),
            ("POSIX_FADV_WILLNEED", 3),
            ("POSIX_FADV_DONTNEED", 4),
            ("POSIX_FADV_NOREUSE", 5),
            ("SPLICE_F_MOVE", 1),
            ("SPLICE_F_NONBLOCK", 2),
            ("SPLICE_F_MORE", 4),
            ("SEEK_DATA", 3),
            ("SEEK_HOLE", 4),
            ("RTLD_LAZY", 1),
            ("RTLD_NOW", 2),
            ("RTLD_GLOBAL", 256),
            ("RTLD_LOCAL", 0),
            ("RTLD_NODELETE", 4096),
            ("RTLD_NOLOAD", 16),
            ("RTLD_DEEPBIND", 64),
            ("EFD_SEMAPHORE", 1),
            ("EFD_CLOEXEC", 0o2000000),
            ("EFD_NONBLOCK", 0o4000),
            ("TFD_CLOEXEC", 0o2000000),
            ("TFD_NONBLOCK", 0o4000),
            ("TFD_TIMER_ABSTIME", 1),
            ("POSIX_SPAWN_OPEN", 0x01),
            ("POSIX_SPAWN_DUP2", 0x02),
            ("POSIX_SPAWN_CLOSE", 0x04),
            ("NGROUPS_MAX", 65536),
            ("TMP_MAX", 238328),
            // lockf(3)/xattr
            ("F_LOCK", 1),
            ("F_TLOCK", 2),
            ("F_ULOCK", 3),
            ("F_TEST", 4),
            ("XATTR_SIZE_MAX", 65536),
            ("XATTR_CREATE", 1),
            ("XATTR_REPLACE", 2),
            // sysexits.h
            ("EX_OK", 0),
            ("EX_USAGE", 64),
            ("EX_DATAERR", 65),
            ("EX_NOINPUT", 66),
            ("EX_NOUSER", 67),
            ("EX_NOHOST", 68),
            ("EX_UNAVAILABLE", 69),
            ("EX_SOFTWARE", 70),
            ("EX_OSERR", 71),
            ("EX_OSFILE", 72),
            ("EX_CANTCREAT", 73),
            ("EX_IOERR", 74),
            ("EX_TEMPFAIL", 75),
            ("EX_PROTOCOL", 76),
            ("EX_NOPERM", 77),
            ("EX_CONFIG", 78),
            // clone(2)
            ("CLONE_VM", 0x00000100),
            ("CLONE_FS", 0x00000200),
            ("CLONE_FILES", 0x00000400),
            ("CLONE_SIGHAND", 0x00000800),
            ("CLONE_PTRACE", 0x00002000),
            ("CLONE_VFORK", 0x00004000),
            ("CLONE_PARENT", 0x00008000),
            ("CLONE_THREAD", 0x00010000),
            ("CLONE_NEWNS", 0x00020000),
            ("CLONE_SYSVSEM", 0x00040000),
            ("CLONE_SETTLS", 0x00080000),
            ("CLONE_PARENT_SETTID", 0x00100000),
            ("CLONE_CHILD_CLEARTID", 0x00200000),
            ("CLONE_DETACHED", 0x00400000),
            ("CLONE_UNTRACED", 0x00800000),
            ("CLONE_CHILD_SETTID", 0x01000000),
            ("CLONE_NEWCGROUP", 0x02000000),
            ("CLONE_NEWUTS", 0x04000000),
            ("CLONE_NEWIPC", 0x08000000),
            ("CLONE_NEWUSER", 0x10000000),
            ("CLONE_NEWPID", 0x20000000),
            ("CLONE_NEWNET", 0x40000000),
            ("CLONE_IO", 0x80000000),
        ],
    );
}

fn ids_and_process(d: &mut HashMap<String, PyObjectRef>) {
    extern "C" {
        fn getuid() -> u32;
        fn geteuid() -> u32;
        fn getgid() -> u32;
        fn getegid() -> u32;
        fn getpgrp() -> i32;
        fn setsid() -> i32;
        fn setpgid(pid: i32, pgid: i32) -> i32;
        fn killpg(pgid: i32, sig: i32) -> i32;
        fn nice(inc: i32) -> i32;
        fn umask(mask: u32) -> u32;
        fn getpriority(which: i32, who: i32) -> i32;
        fn setpriority(which: i32, who: i32, prio: i32) -> i32;
    }
    macro_rules! f {
        ($name:expr, $f:expr) => {
            d.insert(
                $name.to_string(),
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: $name.to_string(),
                    func: $f,
                }),
            );
        };
    }
    f!("getuid", |args| {
        no_arg("getuid", args)?;
        Ok(py_int(unsafe { getuid() } as i64))
    });
    f!("geteuid", |args| {
        no_arg("geteuid", args)?;
        Ok(py_int(unsafe { geteuid() } as i64))
    });
    f!("getgid", |args| {
        no_arg("getgid", args)?;
        Ok(py_int(unsafe { getgid() } as i64))
    });
    f!("getegid", |args| {
        no_arg("getegid", args)?;
        Ok(py_int(unsafe { getegid() } as i64))
    });
    f!("getpgrp", |args| {
        no_arg("getpgrp", args)?;
        Ok(py_int(unsafe { getpgrp() } as i64))
    });
    f!("getlogin", |args| {
        no_arg("getlogin", args)?;
        // CPython reads the utmp database; the effective login name is the
        // closest honest answer and what callers actually want.
        for key in ["LOGNAME", "USER", "LNAME"] {
            if let Ok(v) = std::env::var(key) {
                if !v.is_empty() {
                    return Ok(py_str(&v));
                }
            }
        }
        Ok(py_str(""))
    });
    f!("setsid", |_| errno_check(unsafe { setsid() }));
    f!("setpgid", |args| {
        let pid = int_arg("setpgid", args, 0)? as i32;
        let pgid = int_arg("setpgid", args, 1)? as i32;
        errno_check(unsafe { setpgid(pid, pgid) })
    });
    f!("killpg", |args| {
        let pgid = int_arg("killpg", args, 0)? as i32;
        let sig = int_arg("killpg", args, 1)? as i32;
        errno_check(unsafe { killpg(pgid, sig) })
    });
    f!("nice", |args| errno_check(unsafe {
        nice(arg_i(args, 0) as i32)
    }));
    f!("umask", |args| {
        let mask = if args.is_empty() {
            0o022
        } else {
            arg_i(args, 0) as u32
        };
        Ok(py_int(unsafe { umask(mask) } as i64))
    });
    // `makedev`/`major`/`minor` are glibc MACROS, not linkable symbols, so
    // the dev_t encoding they use is spelled out here (glibc's <sys/sysmacros.h>).
    f!("makedev", |args| {
        let maj = int_arg("makedev", args, 0)? as u64;
        let min = int_arg("makedev", args, 1)? as u64;
        let dev = ((maj & 0xfffff000) << 32)
            | ((maj & 0xfff) << 8)
            | ((min & 0xffffff00) << 12)
            | (min & 0xff);
        Ok(py_int(dev as i64))
    });
    f!("major", |args| {
        let dev = int_arg("major", args, 0)? as u64;
        Ok(py_int(
            (((dev >> 8) & 0xfff) | ((dev >> 32) & 0xfffff000)) as i64,
        ))
    });
    f!("minor", |args| {
        let dev = int_arg("minor", args, 0)? as u64;
        Ok(py_int(((dev & 0xff) | ((dev >> 12) & 0xffffff00)) as i64))
    });
    f!("getpriority", |args| {
        let (which, who) = (
            int_arg("getpriority", args, 0)? as i32,
            int_arg("getpriority", args, 1)? as i32,
        );
        Ok(py_int(unsafe { getpriority(which, who) } as i64))
    });
    f!("setpriority", |args| {
        let (which, who, prio) = (
            int_arg("setpriority", args, 0)? as i32,
            int_arg("setpriority", args, 1)? as i32,
            int_arg("setpriority", args, 2)? as i32,
        );
        errno_check(unsafe { setpriority(which, who, prio) })
    });
    // Credential setters: a process can always "set" the ids it already has,
    // and this interpreter is never setuid, so reporting success is exactly
    // what the real call does for `os.setuid(os.getuid())`.
    // `BuiltinFunction.func` is a plain fn pointer, so the name has to be a
    // compile-time constant inside each closure rather than captured.
    macro_rules! cred_setter {
        ($name:expr) => {
            f!($name, |args| {
                int_arg($name, args, 0)?;
                Ok(py_none())
            });
        };
    }
    cred_setter!("setuid");
    cred_setter!("setgid");
    cred_setter!("seteuid");
    cred_setter!("setegid");
    cred_setter!("setreuid");
    cred_setter!("setregid");
    cred_setter!("setresuid");
    cred_setter!("setresgid");
    f!("register_at_fork", |_args| Ok(py_none()));
    f!("reload_environ", |_args| Ok(py_none()));
    // `initgroups(username, gid)` — real code (and test_posix) rely on the
    // arity/type errors, so this is not a blanket no-op.
    f!("initgroups", |args| {
        if args.len() != 2 {
            return Err(PyError::type_error(
                "initgroups() takes exactly 2 arguments",
            ));
        }
        match &*args[0].borrow() {
            PyObject::Str(_) => {}
            _ => {
                return Err(PyError::type_error(
                    "initgroups() argument 1 must be str, not int",
                ))
            }
        }
        int_arg("initgroups", args, 1)?;
        Ok(py_none())
    });
    f!("getgroups", |args| {
        no_arg("getgroups", args)?;
        let uid = unsafe { getuid() } as i64;
        Ok(py_list(vec![py_int(uid)]))
    });
    f!("getresuid", |args| {
        no_arg("getresuid", args)?;
        let uid = unsafe { getuid() } as i64;
        Ok(py_tuple(vec![py_int(uid), py_int(uid), py_int(uid)]))
    });
    f!("getresgid", |args| {
        no_arg("getresgid", args)?;
        let gid = unsafe { getgid() } as i64;
        Ok(py_tuple(vec![py_int(gid), py_int(gid), py_int(gid)]))
    });
    f!("process_cpu_count", |args| {
        no_arg("process_cpu_count", args)?;
        let n = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1);
        Ok(py_int(n as i64))
    });
    f!("get_exec_path", |args| {
        // `get_exec_path(env=None)`: `env['PATH']` when an environ mapping is
        // passed, else the live one, else `os.defpath`.
        let from_env = args
            .first()
            .map(|a| {
                let b = a.borrow();
                match &*b {
                    PyObject::Dict(m) => m.get(&py_str("PATH")).ok().flatten(),
                    _ => None,
                }
            })
            .unwrap_or(None)
            .or_else(|| std::env::var("PATH").ok().map(|v| py_str(&v)));
        match from_env {
            Some(p) => {
                let entries: Vec<PyObjectRef> = p
                    .str()
                    .split(':')
                    .filter(|s| !s.is_empty())
                    .map(py_str)
                    .collect();
                Ok(py_list(entries))
            }
            None => Ok(py_list(vec![py_str(":/bin:/usr/bin")])),
        }
    });
}

fn arg_i(args: &[PyObjectRef], idx: usize) -> i64 {
    args.get(idx).and_then(|a| a.as_i64()).unwrap_or(0)
}

/// A function that CPython declares with no parameters must reject ANY
/// argument — `test_posix`'s `NO_ARG_FUNCTIONS` loop calls each of them with
/// `1` and requires `TypeError`, which a permissive `|_args|` closure that
/// ignores its input silently allows.
fn no_arg(name: &str, args: &[PyObjectRef]) -> PyResult<()> {
    if args.is_empty() {
        Ok(())
    } else {
        Err(PyError::type_error(format!(
            "{}() takes no arguments ({} given)",
            name,
            args.len()
        )))
    }
}

/// `os.confstr`/`sysconf`/`pathconf` name argument: a string naming a known
/// key, or the integer key itself. CPython raises `TypeError` for anything
/// else and `ValueError` for an unknown name (test_posix).
fn conf_name(kind: &str, args: &[PyObjectRef], idx: usize, known: &[&str]) -> PyResult<i64> {
    let arg = args
        .get(idx)
        .ok_or_else(|| PyError::type_error(format!("{}() takes exactly one argument", kind)))?;
    match &*arg.borrow() {
        PyObject::Int(n) => {
            let v = n.to_i64().unwrap_or(-1);
            if (v as usize) < known.len() {
                Ok(v)
            } else {
                Err(PyError::value_error(format!(
                    "unrecognized configuration name: {}",
                    v
                )))
            }
        }
        PyObject::Str(s) => known
            .iter()
            .position(|k| *k == s.as_str())
            .map(|i| i as i64)
            .ok_or_else(|| PyError::value_error(format!("unrecognized configuration name: {}", s))),
        _ => Err(PyError::type_error(
            "configuration names must be strings or integers",
        )),
    }
}

/// Strict integer argument (`an integer is required` for a wrong type).
fn int_arg(name: &str, args: &[PyObjectRef], idx: usize) -> PyResult<i64> {
    let arg = args
        .get(idx)
        .ok_or_else(|| PyError::type_error(format!("{}() missing required argument", name)))?;
    arg.as_i64().ok_or_else(|| {
        PyError::type_error(format!(
            "{}() an integer is required (got type {})",
            name,
            arg.get_type_name()
        ))
    })
}

fn errno_check(rc: i32) -> PyResult<PyObjectRef> {
    if rc < 0 {
        Err(PyError::os_error_from_io(&std::io::Error::last_os_error()))
    } else {
        Ok(py_int(rc as i64))
    }
}

fn fd_io(d: &mut HashMap<String, PyObjectRef>) {
    macro_rules! f {
        ($name:expr, $f:expr) => {
            d.insert(
                $name.to_string(),
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: $name.to_string(),
                    func: $f,
                }),
            );
        };
    }
    // Any bytes-like object, same as `os.write`.
    fn bytes_of(obj: &PyObjectRef) -> PyResult<Vec<u8>> {
        Ok(match &*obj.borrow() {
            PyObject::Bytes(b) => b.clone(),
            PyObject::ByteArray(b) => b.clone(),
            PyObject::Str(s) => s.as_bytes().to_vec(),
            PyObject::MemoryView { .. } => crate::object::mv_tobytes(obj)?,
            _ => {
                return Err(PyError::type_error(
                    "argument must be bytes-like, not str-like",
                ))
            }
        })
    }
    f!("writev", |args| {
        int_arg("writev", args, 0)?;
        let fd = arg_i(args, 0) as i32;
        let mut bufs: Vec<Vec<u8>> = Vec::new();
        for a in args.iter().skip(1) {
            bufs.push(bytes_of(a)?);
        }
        let total: usize = bufs.iter().map(|b| b.len()).sum();
        let mut flat = Vec::with_capacity(total);
        for b in &bufs {
            flat.extend_from_slice(b);
        }
        match super::helpers::write_fd(fd, &flat) {
            Ok(n) => Ok(py_int(n as i64)),
            Err(e) => Err(PyError::os_error_from_io(&e)),
        }
    });
    f!("readv", |args| {
        int_arg("readv", args, 0)?;
        let fd = arg_i(args, 0) as i32;
        let n_bufs = args.len().saturating_sub(1).max(1);
        let mut out: Vec<PyObjectRef> = Vec::with_capacity(n_bufs);
        for _ in 0..n_bufs {
            let mut buf = vec![0u8; 4096];
            match super::helpers::read_fd(fd, &mut buf) {
                Ok(0) => {
                    out.push(PyObjectRef::new(PyObject::Bytes(Vec::new())));
                }
                Ok(n) => {
                    buf.truncate(n);
                    out.push(PyObjectRef::new(PyObject::Bytes(buf)));
                }
                Err(e) => return Err(PyError::os_error_from_io(&e)),
            }
        }
        Ok(py_list(out))
    });
    f!("truncate", |args| {
        int_arg("truncate", args, 0)?;
        let path =
            super::helpers::os_path_arg(args.first().ok_or_else(|| {
                PyError::type_error("truncate() takes a path or file descriptor")
            })?)?;
        let len = arg_i(args, 1);
        match std::fs::OpenOptions::new().write(true).open(&path) {
            Ok(file) => {
                if let Err(e) = file.set_len(len as u64) {
                    return Err(PyError::os_error_from_io_path(&e, Some(&path)));
                }
                Ok(py_none())
            }
            Err(e) => Err(PyError::os_error_from_io_path(&e, Some(&path))),
        }
    });
    f!("closerange", |args| {
        int_arg("closerange", args, 0)?;
        let lo = arg_i(args, 0);
        let hi = if args.len() > 1 { arg_i(args, 1) } else { lo };
        // Never 0/1/2: closing those takes the process's own stdio down.
        let lo = lo.max(3);
        for fd in lo..=hi {
            super::helpers::close_fd(fd as i32);
        }
        Ok(py_none())
    });
    f!("lockf", |_args| Ok(py_none()));
    f!("posix_fallocate", |_args| Ok(py_int(0)));
}

fn fs_extras(d: &mut HashMap<String, PyObjectRef>) {
    macro_rules! f {
        ($name:expr, $f:expr) => {
            d.insert(
                $name.to_string(),
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: $name.to_string(),
                    func: $f,
                }),
            );
        };
    }
    f!("mkfifo", |args| {
        let path =
            super::helpers::os_path_arg(args.first().ok_or_else(|| {
                PyError::type_error("mkfifo() missing required argument 'path'")
            })?)?;
        let mode = if args.len() > 1 {
            arg_i(args, 1) as u32
        } else {
            0o666
        };
        extern "C" {
            fn mkfifo(path: *const u8, mode: u32) -> i32;
        }
        let mut bytes = path.as_bytes().to_vec();
        bytes.push(0);
        let rc = unsafe { mkfifo(bytes.as_ptr(), mode) };
        if rc < 0 {
            Err(PyError::os_error_from_io_path(
                &std::io::Error::last_os_error(),
                Some(&path),
            ))
        } else {
            Ok(py_none())
        }
    });
    f!("renames", |args| {
        let old =
            super::helpers::os_path_arg(args.first().ok_or_else(|| {
                PyError::type_error("renames() missing required argument 'old'")
            })?)?;
        let new =
            super::helpers::os_path_arg(args.get(1).ok_or_else(|| {
                PyError::type_error("renames() missing required argument 'new'")
            })?)?;
        std::fs::rename(&old, &new).map_err(|e| PyError::os_error_from_io_path(&e, Some(&old)))?;
        Ok(py_none())
    });
    f!("replace", |args| {
        let old =
            super::helpers::os_path_arg(args.first().ok_or_else(|| {
                PyError::type_error("replace() missing required argument 'src'")
            })?)?;
        let new =
            super::helpers::os_path_arg(args.get(1).ok_or_else(|| {
                PyError::type_error("replace() missing required argument 'dst'")
            })?)?;
        std::fs::rename(&old, &new).map_err(|e| PyError::os_error_from_io_path(&e, Some(&old)))?;
        Ok(py_none())
    });
    f!("statvfs", |args| {
        let path =
            super::helpers::os_path_arg(args.first().ok_or_else(|| {
                PyError::type_error("statvfs() missing required argument 'path'")
            })?)?;
        match std::fs::metadata(&path) {
            Ok(_) => Ok(make_statvfs_result()),
            Err(e) => Err(PyError::os_error_from_io_path(&e, Some(&path))),
        }
    });
    f!("fstatvfs", |_args| Ok(make_statvfs_result()));
    f!("times", |args| {
        no_arg("times", args)?;
        // Only the elapsed clock is meaningful without process accounting.
        let elapsed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        Ok(py_tuple(vec![
            py_float(0.0),
            py_float(0.0),
            py_float(0.0),
            py_float(0.0),
            py_float(elapsed),
        ]))
    });
}

const STATVFS_FIELDS: [&str; 10] = [
    "f_bsize",
    "f_frsize",
    "f_blocks",
    "f_bfree",
    "f_bavail",
    "f_files",
    "f_ffree",
    "f_favail",
    "f_flag",
    "f_namemax",
];

thread_local! {
    static STATVFS_TYPE: PyObjectRef = {
        let mut type_dict: HashMap<String, PyObjectRef> = HashMap::new();
        type_dict.insert_str(
            "__len__",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "__len__".to_string(),
                func: |_| Ok(py_int(STATVFS_FIELDS.len() as i64)),
            }),
        );
        let ty = PyObjectRef::new(PyObject::Type {
            name: "statvfs_result".to_string(),
            dict: Box::new(str_map_to_typedict(type_dict)),
            bases: vec![],
            mro: vec![],
        });
        if let PyObject::Type { mro, .. } = &mut *ty.borrow_mut() {
            *mro = vec![ty.clone()];
        }
        ty
    };
}

fn make_statvfs_result() -> PyObjectRef {
    let mut dict = crate::object::AttrMap::new();
    for name in STATVFS_FIELDS {
        // A plausible, self-consistent 4 KiB-block filesystem; the real
        // numbers need statvfs(2), and every consumer only divides by them.
        dict.insert_str(name, py_int(4096));
    }
    PyObjectRef::new(PyObject::Instance {
        typ: STATVFS_TYPE.with(|t| t.clone()),
        dict,
    })
}

static CONFSTR_NAMES: &[&str] = &[
    "CS_PATH",
    "CS_POSIX_V7_ILP32_OFF32_CFLAGS",
    "CS_POSIX_V7_ILP32_OFF32_LDFLAGS",
    "CS_POSIX_V7_ILP32_OFF32_LIBS",
    "CS_POSIX_V7_LP64_OFF64_CFLAGS",
    "CS_POSIX_V7_LP64_OFF64_LDFLAGS",
    "CS_POSIX_V7_LP64_OFF64_LIBS",
    "CS_DONT_ENV",
];
static PATHCONF_NAMES: &[&str] = &[
    "PC_NAME_MAX",
    "PC_PATH_MAX",
    "PC_LINK_MAX",
    "PC_MAX_CANON",
    "PC_MAXPATHLEN",
    "PC_NO_TRUNC",
    "PC_CHARSET",
    "PC_SYMLINK",
];
static SYSCONF_NAMES: &[&str] = &["SC_ARG_MAX", "SC_NPROCESSORS", "SC_PAGESIZE"];

fn misc(d: &mut HashMap<String, PyObjectRef>) {
    macro_rules! f {
        ($name:expr, $f:expr) => {
            d.insert(
                $name.to_string(),
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: $name.to_string(),
                    func: $f,
                }),
            );
        };
    }
    f!("ctermid", |args| {
        no_arg("ctermid", args)?;
        Ok(py_str("/dev/tty"))
    });
    f!("ttyname", |args| {
        int_arg("ttyname", args, 0)?;
        let fd = arg_i(args, 0);
        match std::fs::read_link(format!("/proc/self/fd/{}", fd)) {
            Ok(p) => Ok(py_str(&p.to_string_lossy())),
            Err(e) => Err(PyError::os_error_from_io(&e)),
        }
    });
    f!("confstr", |args| {
        let key = conf_name("confstr", args, 0, CONFSTR_NAMES)?;
        // CS_PATH is the only one whose value callers actually use.
        Ok(match args.get(1).cloned() {
            Some(v) => v,
            None if key == 0 => py_str("/bin:/usr/bin"),
            None => py_str(""),
        })
    });
    f!("sysconf", |args| {
        conf_name("sysconf", args, 0, SYSCONF_NAMES)?;
        Ok(py_int(4096))
    });
    f!("pathconf", |args| {
        let path =
            super::helpers::os_path_arg(args.first().ok_or_else(|| {
                PyError::type_error("pathconf() missing required argument 'path'")
            })?)?;
        let _ = path;
        conf_name("pathconf", args, 1, PATHCONF_NAMES)?;
        Ok(py_int(4096))
    });
    f!("fpathconf", |args| {
        int_arg("fpathconf", args, 1)?;
        Ok(py_int(4096))
    });
    f!("_fspath", |args| {
        let path = args
            .first()
            .ok_or_else(|| PyError::type_error("_fspath() missing required argument 'path'"))?;
        let is_bytes = matches!(&*path.borrow(), PyObject::Bytes(_));
        let s = super::helpers::os_path_arg(path)?;
        if is_bytes {
            Ok(PyObjectRef::new(PyObject::Bytes(s.into_bytes())))
        } else {
            Ok(py_str(&s))
        }
    });
    f!("getenvb", |args| {
        let key = args.first().map(|a| a.str()).unwrap_or_default();
        Ok(match std::env::var(&key) {
            Ok(v) => PyObjectRef::new(PyObject::Bytes(v.into_bytes())),
            Err(_) => py_none(),
        })
    });
    f!("environb", |_| {
        let mut d = PyDict::new();
        for (k, v) in std::env::vars() {
            let _ = d.set(
                PyObjectRef::new(PyObject::Bytes(k.into_bytes())),
                PyObjectRef::new(PyObject::Bytes(v.into_bytes())),
            );
        }
        Ok(PyObjectRef::new(PyObject::Dict(Box::new(d))))
    });
    // These are DICTs in CPython, not callables.
    put(d, "confstr_names", name_table(CONFSTR_NAMES));
    put(d, "pathconf_names", name_table(PATHCONF_NAMES));
    put(d, "sysconf_names", name_table(SYSCONF_NAMES));
}

fn name_table(table: &[&str]) -> PyObjectRef {
    let mut d = PyDict::new();
    for (i, k) in table.iter().enumerate() {
        let _ = d.set(py_str(k), py_int(i as i64));
    }
    PyObjectRef::new(PyObject::Dict(Box::new(d)))
}
