//! File-descriptor level `os` entry points that need raw syscalls.
//!
//! No libc binding exists in this build, so each function issues its syscall
//! through inline asm (x86_64 Linux) and reports `ENOSYS`-flavoured errors
//! elsewhere. They live together because they are all the same shape: a
//! syscall number, an argument list, and an errno turned into an `OSError`.

use crate::object::*;
use std::collections::HashMap;

// --- x86_64 Linux syscall numbers -----------------------------------------
const SYS_FCNTL: i64 = 72;
const SYS_FSYNC: i64 = 74;
const SYS_FDATASYNC: i64 = 75;
const SYS_TRUNCATE: i64 = 76;
const SYS_FTRUNCATE: i64 = 77;
const SYS_FCHDIR: i64 = 81;
const SYS_LCHOWN: i64 = 94;
const SYS_MKNOD: i64 = 133;
const SYS_LGETXATTR: i64 = 192;
const SYS_FGETXATTR: i64 = 193;
const SYS_GETXATTR: i64 = 191;
const SYS_LSETXATTR: i64 = 189;
const SYS_FSETXATTR: i64 = 190;
const SYS_SETXATTR: i64 = 188;
const SYS_LLISTXATTR: i64 = 195;
const SYS_FLISTXATTR: i64 = 196;
const SYS_LISTXATTR: i64 = 194;
const SYS_LREMOVEXATTR: i64 = 198;
const SYS_FREMOVEXATTR: i64 = 199;
const SYS_REMOVEXATTR: i64 = 197;
const SYS_SENDFILE: i64 = 40;
const SYS_FADVISE64: i64 = 221;
const SYS_FALLOCATE: i64 = 285;
const SYS_SYNC: i64 = 162;
const SYS_SYNCFS: i64 = 306;
const SYS_DUP: i64 = 32;
const SYS_DUP2: i64 = 33;
const SYS_PIPE2: i64 = 293;

// --- constants ------------------------------------------------------------
const F_GETFD: i64 = 1;
const F_SETFD: i64 = 2;
const FD_CLOEXEC: i64 = 1;
const F_DUPFD: i64 = 0;
const F_DUPFD_CLOEXEC: i64 = 0o20000000;
const F_GETFL: i64 = 3;
const F_SETFL: i64 = 4;
const AT_FDCWD: i64 = -100;
const AT_SYMLINK_NOFOLLOW: i64 = 0x100;
const AT_REMOVEDIR: i64 = 0x200;
const AT_EMPTY_PATH: i64 = 0x1000;
const O_CLOEXEC: i64 = 0o2000000;
const SEEK_SET: i64 = 0;
const SEEK_CUR: i64 = 1;
const SEEK_END: i64 = 2;

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod raw {
    /// `syscall(number, a1..a6)`, returning libc's convention (negative
    /// errno on failure).
    ///
    /// # Safety
    /// Arguments must match the syscall's real signature; a wrong pointer is
    /// undefined behaviour, as with libc.
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn call(n: i64, a1: i64, a2: i64, a3: i64, a4: i64, a5: i64, a6: i64) -> i64 {
        let ret: i64;
        std::arch::asm!(
            "syscall",
            inlateout("rax") n => ret,
            in("rdi") a1,
            in("rsi") a2,
            in("rdx") a3,
            in("r10") a4,
            in("r8") a5,
            in("r9") a6,
            lateout("rcx") _,
            lateout("r11") _,
        );
        ret
    }
}

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
mod raw {
    pub unsafe fn call(_n: i64, _a1: i64, _a2: i64, _a3: i64, _a4: i64, _a5: i64, _a6: i64) -> i64 {
        -(38i64) // ENOSYS
    }
}

fn sys(n: i64, a1: i64, a2: i64, a3: i64) -> i64 {
    // SAFETY: arguments match the syscall named at each call site.
    unsafe { raw::call(n, a1, a2, a3, 0, 0, 0) }
}

fn sys4(n: i64, a1: i64, a2: i64, a3: i64, a4: i64) -> i64 {
    // SAFETY: as above.
    unsafe { raw::call(n, a1, a2, a3, a4, 0, 0) }
}

fn sys5(n: i64, a1: i64, a2: i64, a3: i64, a4: i64, a5: i64) -> i64 {
    // SAFETY: as above.
    unsafe { raw::call(n, a1, a2, a3, a4, a5, 0) }
}

fn errno_of(ret: i64) -> i32 {
    if ret < 0 && ret > -4096 {
        (-ret) as i32
    } else {
        0
    }
}

fn check(ret: i64) -> Result<i64, PyError> {
    if ret >= 0 {
        Ok(ret)
    } else {
        Err(PyError::os_error_from_io(
            &std::io::Error::from_raw_os_error(errno_of(ret)),
        ))
    }
}

fn py_str_bytes(data: Vec<u8>) -> PyObjectRef {
    PyObjectRef::new(PyObject::Bytes(data))
}

fn cstr(s: &str) -> Vec<u8> {
    let mut v = s.as_bytes().to_vec();
    v.push(0);
    v
}

fn arg_str(args: &[PyObjectRef], idx: usize) -> String {
    args.get(idx)
        .map(|v| match &*v.borrow() {
            PyObject::Str(s) => s.to_string(),
            other => other.repr(),
        })
        .unwrap_or_default()
}

fn arg_int(args: &[PyObjectRef], idx: usize, default: i64) -> i64 {
    args.get(idx).and_then(|a| a.as_i64()).unwrap_or(default)
}

/// `getxattr`-family: with no buffer the kernel returns the required size,
/// which is the two-call protocol the stdlib and the tests use.
fn xattr_get(num: i64, target: i64, name: &str) -> Result<PyObjectRef, PyError> {
    let name_c = cstr(name);
    let size = check(sys4(num, target, name_c.as_ptr() as i64, 0, 0))?;
    let size = size as usize;
    if size == 0 {
        return Ok(py_str_bytes(Vec::new()));
    }
    let mut buf = vec![0u8; size];
    let ret = check(sys4(
        num,
        target,
        name_c.as_ptr() as i64,
        buf.as_mut_ptr() as i64,
        size as i64,
    ))?;
    buf.truncate(ret as usize);
    Ok(py_str_bytes(buf))
}

fn xattr_list(num: i64, target: i64) -> Result<PyObjectRef, PyError> {
    let size = check(sys4(num, target, 0, 0, 0))?;
    let size = size as usize;
    if size == 0 {
        return Ok(py_str_bytes(Vec::new()));
    }
    let mut buf = vec![0u8; size];
    let ret = check(sys4(num, target, buf.as_mut_ptr() as i64, size as i64, 0))?;
    buf.truncate(ret as usize);
    // The kernel returns NUL-separated names; CPython hands back a list.
    let names: Vec<PyObjectRef> = buf
        .split(|b| *b == 0)
        .filter(|part| !part.is_empty())
        .map(|part| py_str(&String::from_utf8_lossy(part).into_owned()))
        .collect();
    Ok(py_list(names))
}

fn xattr_set(num: i64, target: i64, name: &str, value: &str, flags: i64) -> PyResult<()> {
    let name_c = cstr(name);
    let value_c = cstr(value);
    // The length must exclude the NUL `cstr` appends, or the stored value
    // would carry a trailing NUL byte.
    let ret = sys5(
        num,
        target,
        name_c.as_ptr() as i64,
        value_c.as_ptr() as i64,
        (value_c.len() - 1) as i64,
        flags,
    );
    check(ret).map(|_| ())
}

fn xattr_remove(num: i64, target: i64, name: &str) -> PyResult<()> {
    let name_c = cstr(name);
    check(sys4(num, target, name_c.as_ptr() as i64, 0, 0)).map(|_| ())
}

/// Register the fd-level `os` functions this module provides.
pub fn install(d: &mut HashMap<String, PyObjectRef>) {
    // `os.set_inheritable` / `os.get_inheritable` drive FD_CLOEXEC through
    // `fcntl(F_SETFD/F_GETFD)`. `subprocess`'s `pass_fds` support is built on
    // them, and `test_subprocess` calls them directly — without them the
    // vendored `subprocess.py` raises `AttributeError` on its way to spawning.
    d.insert_str(
        "set_inheritable",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "set_inheritable".to_string(),
            func: |args: &[PyObjectRef]| {
                let fd = arg_int(args, 0, -1) as i32;
                let inheritable = args.get(1).map(|v| v.truthy()).unwrap_or(false);
                let flags = check(sys(SYS_FCNTL, fd as i64, F_GETFD, 0))? as i32;
                let updated = if inheritable {
                    flags & !(FD_CLOEXEC as i32)
                } else {
                    flags | FD_CLOEXEC as i32
                };
                check(sys(SYS_FCNTL, fd as i64, F_SETFD, updated as i64)).map(|_| py_none())
            },
        }),
    );

    d.insert_str(
        "get_inheritable",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "get_inheritable".to_string(),
            func: |args: &[PyObjectRef]| {
                let fd = arg_int(args, 0, -1) as i32;
                let flags = check(sys(SYS_FCNTL, fd as i64, F_GETFD, 0))? as i32;
                Ok(py_bool(flags & FD_CLOEXEC as i32 == 0))
            },
        }),
    );

    d.insert_str(
        "dup",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "dup".to_string(),
            func: |args: &[PyObjectRef]| {
                let fd = arg_int(args, 0, -1) as i32;
                check(sys(SYS_DUP, fd as i64, 0, 0)).map(py_int)
            },
        }),
    );

    d.insert_str(
        "dup2",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "dup2".to_string(),
            func: |args: &[PyObjectRef]| {
                let old = arg_int(args, 0, -1) as i32;
                let new = arg_int(args, 1, -1) as i32;
                check(sys(SYS_DUP2, old as i64, new as i64, 0)).map(py_int)
            },
        }),
    );

    d.insert_str(
        "pipe2",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "pipe2".to_string(),
            func: |args: &[PyObjectRef]| {
                let flags = arg_int(args, 0, 0);
                let mut fds = [0i32; 2];
                let ret = sys5(SYS_PIPE2, fds.as_mut_ptr() as i64, flags, 0, 0, 0);
                check(ret)?;
                Ok(py_tuple(vec![py_int(fds[0] as i64), py_int(fds[1] as i64)]))
            },
        }),
    );

    // `os.abort()` raises SIGABRT; several stdlib modules use it to abort a
    // child on purpose (`multiprocessing`, `asyncio` child watchers).
    d.insert_str(
        "abort",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "abort".to_string(),
            func: |_args: &[PyObjectRef]| {
                let pid = crate::modules::posixsubprocess::self_pid() as i64;
                check(sys(62, pid, 6, 0)).map(|_| py_none())
            },
        }),
    );

    d.insert_str(
        "truncate",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "truncate".to_string(),
            func: |args: &[PyObjectRef]| {
                let path = cstr(&arg_str(args, 0));
                let len = arg_int(args, 1, 0);
                check(sys(SYS_TRUNCATE, path.as_ptr() as i64, len, 0)).map(|_| py_none())
            },
        }),
    );

    d.insert_str(
        "ftruncate",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "ftruncate".to_string(),
            func: |args: &[PyObjectRef]| {
                let fd = arg_int(args, 0, -1) as i32;
                let len = arg_int(args, 1, 0);
                check(sys(SYS_FTRUNCATE, fd as i64, len, 0)).map(|_| py_none())
            },
        }),
    );

    d.insert_str(
        "fsync",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "fsync".to_string(),
            func: |args: &[PyObjectRef]| {
                let fd = arg_int(args, 0, -1) as i32;
                check(sys(SYS_FSYNC, fd as i64, 0, 0)).map(|_| py_none())
            },
        }),
    );

    d.insert_str(
        "fdatasync",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "fdatasync".to_string(),
            func: |args: &[PyObjectRef]| {
                let fd = arg_int(args, 0, -1) as i32;
                check(sys(SYS_FDATASYNC, fd as i64, 0, 0)).map(|_| py_none())
            },
        }),
    );

    d.insert_str(
        "sync",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "sync".to_string(),
            func: |_args: &[PyObjectRef]| check(sys(SYS_SYNC, 0, 0, 0)).map(|_| py_none()),
        }),
    );

    d.insert_str(
        "syncfs",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "syncfs".to_string(),
            func: |args: &[PyObjectRef]| {
                let fd = arg_int(args, 0, -1) as i32;
                check(sys(SYS_SYNCFS, fd as i64, 0, 0)).map(|_| py_none())
            },
        }),
    );

    d.insert_str(
        "fchdir",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "fchdir".to_string(),
            func: |args: &[PyObjectRef]| {
                let fd = arg_int(args, 0, -1) as i32;
                check(sys(SYS_FCHDIR, fd as i64, 0, 0)).map(|_| py_none())
            },
        }),
    );

    d.insert_str(
        "lchown",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "lchown".to_string(),
            func: |args: &[PyObjectRef]| {
                let path = cstr(&arg_str(args, 0));
                let uid = arg_int(args, 1, -1);
                let gid = arg_int(args, 2, -1);
                check(sys(SYS_LCHOWN, path.as_ptr() as i64, uid, gid)).map(|_| py_none())
            },
        }),
    );

    d.insert_str(
        "mknod",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "mknod".to_string(),
            func: |args: &[PyObjectRef]| {
                let path = cstr(&arg_str(args, 0));
                let mode = arg_int(args, 1, 0o600);
                let dev = arg_int(args, 2, 0);
                check(sys(SYS_MKNOD, path.as_ptr() as i64, mode, dev)).map(|_| py_none())
            },
        }),
    );

    // --- extended attributes ---------------------------------------------
    macro_rules! xattr_family {
        ($get:literal, $getnum:expr, $list:literal, $listnum:expr, $set:literal, $setnum:expr,
         $remove:literal, $removenum:expr) => {
            d.insert_str(
                $get,
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: $get.to_string(),
                    func: |args: &[PyObjectRef]| {
                        let target = cstr(&arg_str(args, 0));
                        xattr_get($getnum, target.as_ptr() as i64, &arg_str(args, 1))
                    },
                }),
            );
            d.insert_str(
                $list,
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: $list.to_string(),
                    func: |args: &[PyObjectRef]| {
                        let target = cstr(&arg_str(args, 0));
                        xattr_list($listnum, target.as_ptr() as i64)
                    },
                }),
            );
            d.insert_str(
                $set,
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: $set.to_string(),
                    func: |args: &[PyObjectRef]| {
                        let target = cstr(&arg_str(args, 0));
                        let value = arg_str(args, 2);
                        let flags = arg_int(args, 3, 0);
                        xattr_set(
                            $setnum,
                            target.as_ptr() as i64,
                            &arg_str(args, 1),
                            &value,
                            flags,
                        )
                        .map(|_| py_none())
                    },
                }),
            );
            d.insert_str(
                $remove,
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: $remove.to_string(),
                    func: |args: &[PyObjectRef]| {
                        let target = cstr(&arg_str(args, 0));
                        xattr_remove($removenum, target.as_ptr() as i64, &arg_str(args, 1))
                            .map(|_| py_none())
                    },
                }),
            );
        };
    }
    xattr_family!(
        "getxattr",
        SYS_GETXATTR,
        "listxattr",
        SYS_LISTXATTR,
        "setxattr",
        SYS_SETXATTR,
        "removexattr",
        SYS_REMOVEXATTR
    );
    xattr_family!(
        "lgetxattr",
        SYS_LGETXATTR,
        "llistxattr",
        SYS_LLISTXATTR,
        "lsetxattr",
        SYS_LSETXATTR,
        "lremovexattr",
        SYS_LREMOVEXATTR
    );

    // The `f*` variants take a descriptor instead of a path, so the first
    // argument is passed straight through as an integer. Each one is written
    // out because `BuiltinFunction.func` is a plain `fn` pointer and cannot
    // capture its syscall number.
    d.insert_str(
        "fgetxattr",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "fgetxattr".to_string(),
            func: |args: &[PyObjectRef]| {
                let fd = arg_int(args, 0, -1);
                xattr_get(SYS_FGETXATTR, fd, &arg_str(args, 1))
            },
        }),
    );
    d.insert_str(
        "flistxattr",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "flistxattr".to_string(),
            func: |args: &[PyObjectRef]| xattr_list(SYS_FLISTXATTR, arg_int(args, 0, -1)),
        }),
    );
    d.insert_str(
        "fsetxattr",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "fsetxattr".to_string(),
            func: |args: &[PyObjectRef]| {
                let value = arg_str(args, 2);
                xattr_set(
                    SYS_FSETXATTR,
                    arg_int(args, 0, -1),
                    &arg_str(args, 1),
                    &value,
                    arg_int(args, 3, 0),
                )
                .map(|_| py_none())
            },
        }),
    );
    d.insert_str(
        "fremovexattr",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "fremovexattr".to_string(),
            func: |args: &[PyObjectRef]| {
                xattr_remove(SYS_FREMOVEXATTR, arg_int(args, 0, -1), &arg_str(args, 1))
                    .map(|_| py_none())
            },
        }),
    );

    // `os.sendfile(out_fd, in_fd, offset, count)` — with `offset=None` the
    // kernel advances the input file position, which is a NULL offset pointer.
    d.insert_str(
        "sendfile",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "sendfile".to_string(),
            func: |args: &[PyObjectRef]| {
                let out_fd = arg_int(args, 0, -1);
                let in_fd = arg_int(args, 1, -1);
                let count = arg_int(args, 3, 0);
                let offset = match args.get(2) {
                    Some(v) if !matches!(&*v.borrow(), PyObject::None) => v.as_i64(),
                    _ => None,
                };
                let mut off = offset.unwrap_or(0);
                let ret = match offset {
                    Some(_) => sys5(
                        SYS_SENDFILE,
                        out_fd,
                        in_fd,
                        &mut off as *mut i64 as i64,
                        count,
                        0,
                    ),
                    None => sys4(SYS_SENDFILE, out_fd, in_fd, 0, count),
                };
                check(ret).map(py_int)
            },
        }),
    );

    d.insert_str(
        "posix_fadvise",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "posix_fadvise".to_string(),
            func: |args: &[PyObjectRef]| {
                let fd = arg_int(args, 0, -1);
                let offset = arg_int(args, 1, 0);
                let length = arg_int(args, 2, 0);
                let advice = arg_int(args, 3, 0);
                check(sys5(SYS_FADVISE64, fd, offset, length, advice, 0)).map(|_| py_none())
            },
        }),
    );

    d.insert_str(
        "fallocate",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "fallocate".to_string(),
            func: |args: &[PyObjectRef]| {
                let fd = arg_int(args, 0, -1);
                let mode = arg_int(args, 1, 0);
                let offset = arg_int(args, 2, 0);
                let length = arg_int(args, 3, 0);
                check(sys5(SYS_FALLOCATE, fd, mode, offset, length, 0)).map(|_| py_none())
            },
        }),
    );

    // `O_CLOEXEC` and friends: `subprocess`'s `pass_fds`/pipe plumbing and
    // `os.pipe2` callers reach for these by name.
    d.insert_str("O_CLOEXEC", py_int(O_CLOEXEC as i64));
    d.insert_str("F_GETFD", py_int(F_GETFD));
    d.insert_str("F_SETFD", py_int(F_SETFD));
    d.insert_str("F_DUPFD", py_int(F_DUPFD));
    d.insert_str("F_DUPFD_CLOEXEC", py_int(F_DUPFD_CLOEXEC));
    d.insert_str("F_GETFL", py_int(F_GETFL));
    d.insert_str("F_SETFL", py_int(F_SETFL));
    d.insert_str("AT_FDCWD", py_int(AT_FDCWD));
    d.insert_str("AT_SYMLINK_NOFOLLOW", py_int(AT_SYMLINK_NOFOLLOW));
    d.insert_str("AT_REMOVEDIR", py_int(AT_REMOVEDIR));
    d.insert_str("AT_EMPTY_PATH", py_int(AT_EMPTY_PATH));
    d.insert_str("SEEK_SET", py_int(SEEK_SET));
    d.insert_str("SEEK_CUR", py_int(SEEK_CUR));
    d.insert_str("SEEK_END", py_int(SEEK_END));
}

/// Clear `FD_CLOEXEC` on `fd` — what CPython's `make_inheritable` does for
/// `subprocess(pass_fds=...)` just before `exec`.
pub fn clear_cloexec(fd: i32) -> bool {
    let flags = sys(SYS_FCNTL, fd as i64, F_GETFD, 0);
    if flags < 0 {
        return false;
    }
    sys(
        SYS_FCNTL,
        fd as i64,
        F_SETFD,
        (flags as i32 & !(FD_CLOEXEC as i32)) as i64,
    ) >= 0
}
