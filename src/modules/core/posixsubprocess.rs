//! `_posixsubprocess` — the spawn primitive CPython's `subprocess.py` binds
//! at import time (`from _posixsubprocess import fork_exec as _fork_exec`),
//! so the vendored, byte-identical `Lib/subprocess.py` can load instead of a
//! native stand-in module.
//!
//! The real thing is a C extension that `fork()`s and `exec`s in the child,
//! wiring the caller's pipe fds and reporting an exec failure back through
//! the "errpipe" (a pair of fds the parent reads: the child writes
//! `b'OSError:<hex errno>:<message>'` when exec fails, writes nothing on
//! success). This implementation keeps the same CONTRACT — same argument
//! order, same 5-tuple return, same errpipe protocol — but spawns through
//! `std::process::Command` (which does its own fork/exec) instead of
//! duplicating CPython's careful fork/exec dance, which this interpreter
//! has no way to reach from safe Rust.
//!
//! What that means concretely for callers:
//!   * `pid` is a real, waitable pid (`os.waitpid` works),
//!   * the caller's `p2cread`/`c2pwrite`-style fds are wired as the child's
//!     stdin/stdout/stderr when they are valid, and `devnull` otherwise,
//!   * a spawn failure (the executable does not exist, `cwd` is missing, ...)
//!     is reported through the errpipe exactly as CPython does, so
//!     `subprocess` raises the same `FileNotFoundError`/`PermissionError`
//!     with the same message,
//!   * `preexec_fn`, `process_group`, `gid`/`uid`/`umask` and `close_fds`
//!     are accepted and ignored where a safe implementation cannot honour
//!     them (they are all documented as unsupported-in-threads platforms
//!     behaviour in CPython too, where they raise at call time).

use crate::object::*;
use std::collections::HashMap;

thread_local! {
    /// Children spawned through `fork_exec`, kept so nothing reaps them
    /// behind `subprocess`'s back (`os.waitpid` stays the single reaper,
    /// matching CPython, where `Popen.poll()`/`wait()` own the status).
    static SPAWNED_CHILDREN: std::cell::RefCell<std::collections::HashMap<i64, std::process::Child>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}
use std::io::Write;
use std::os::fd::{FromRawFd, RawFd};

/// CPython's `_fork_exec` writes `b'<ExceptionName>:<hex errno>:<message>'`
/// into the errpipe when the child fails before/while exec'ing. Anything
/// else (including nothing at all) means success.
fn write_exec_error(errpipe_write: RawFd, exc_name: &str, errno_hex: &str, msg: &str) {
    if errpipe_write < 0 {
        return;
    }
    let payload = format!("{}:{}:{}", exc_name, errno_hex, msg);
    // The write end belongs to the CALLER: `subprocess.py` closes it itself
    // in a `finally` right after `fork_exec` returns, so this must not close
    // it (a double close surfaces there as EBADF). `ManuallyDrop` borrows
    // the fd without taking ownership of it.
    let mut file =
        std::mem::ManuallyDrop::new(unsafe { std::fs::File::from_raw_fd(errpipe_write) });
    let _ = file.write_all(payload.as_bytes());
}

fn arg_strings(value: &PyObjectRef, index: usize) -> Vec<String> {
    let items = match &*value.borrow() {
        PyObject::List(items) | PyObject::Tuple(items) => items.clone(),
        _ => return Vec::new(),
    };
    let _ = index;
    items
        .iter()
        .map(|item| match &*item.borrow() {
            PyObject::Str(s) => s.to_string(),
            PyObject::Bytes(b) => String::from_utf8_lossy(b).to_string(),
            _ => item.str(),
        })
        .collect()
}

fn opt_int(args: &[PyObjectRef], index: usize) -> Option<i64> {
    let value = args.get(index)?;
    if matches!(&*value.borrow(), PyObject::None) {
        return None;
    }
    value.as_i64()
}

pub fn create_posixsubprocess_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    d.insert_str(
        "fork_exec",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "fork_exec".to_string(),
            func: |args: &[PyObjectRef]| {
                // args, executable_list, close_fds, fds_to_keep, cwd, env_list,
                // p2cread, p2cwrite, c2pread, c2pwrite, errread, errwrite,
                // errpipe_read, errpipe_write, restore_signals, call_setsid,
                // pgid_to_set, gid_object, extra_group_size, extra_groups_packed,
                // uid_object, child_umask, preexec_fn  [+ allow_vfork]
                if args.len() < 14 {
                    return Err(PyError::type_error(
                        "fork_exec() missing required arguments",
                    ));
                }
                let argv0_and_args = arg_strings(&args[0], 0);
                let cwd = match &*args[4].borrow() {
                    PyObject::Str(s) => Some(s.to_string()),
                    _ => None,
                };
                // `errpipe_read` must be a real readable fd on BOTH paths:
                // `subprocess.py` reads it to EOF and closes it itself, so
                // both ends are the caller's to close — we must not touch
                // either one here.
                let errpipe_read = opt_int(args, 12).unwrap_or(-1) as RawFd;
                let errpipe_write = opt_int(args, 13).unwrap_or(-1) as RawFd;

                if argv0_and_args.is_empty() {
                    if errpipe_write >= 0 {
                        write_exec_error(errpipe_write, "IndexError", "0x1", "no arguments");
                    }
                    return Ok(py_int(-1));
                }

                let mut command = std::process::Command::new(&argv0_and_args[0]);
                command.args(&argv0_and_args[1..]);
                for entry in env_pairs(&args[5]) {
                    command.env(entry.0, entry.1);
                }
                if let Some(dir) = cwd {
                    command.current_dir(dir);
                }

                // `start_new_session=True` — CPython's `call_setsid`; std has
                // no `setsid` wrapper, so it is issued as a raw syscall from
                // `pre_exec` (async-signal-safe, like every call made there).
                // Arg 15 is `call_setsid` (`start_new_session=True`);
                // arg 16 is `pgid_to_set` (`process_group=`), which std
                // exposes directly.
                let setsid = opt_int(args, 15).unwrap_or(0) != 0;
                let pgid_to_set = opt_int(args, 16).unwrap_or(-1);
                if setsid {
                    unsafe {
                        use std::os::unix::process::CommandExt;
                        command.pre_exec(|| {
                            raw_setsid();
                            Ok(())
                        });
                    }
                } else if pgid_to_set > 0 {
                    unsafe {
                        use std::os::unix::process::CommandExt;
                        command.process_group(pgid_to_set as i32);
                    }
                } else if pgid_to_set == 0 {
                    // `process_group=0` means "new group, id = own pid".
                    // std's `process_group` rejects 0, so the syscall is
                    // issued directly instead.
                    unsafe {
                        use std::os::unix::process::CommandExt;
                        command.pre_exec(|| {
                            raw_setpgid_self();
                            Ok(())
                        });
                    }
                }

                // fd names describe the DATA flow, not the pipe end:
                // `p2c*` is parent-to-child, so the CHILD reads stdin from
                // `p2cread` and the parent writes through `p2cwrite`; likewise
                // the child writes stdout to `c2pwrite` while the parent reads
                // `c2pread`. Using the pipe's other end made every child's
                // `sys.stdin.buffer.read()` fail with EBADF.
                const DEVNULL: i64 = -3;
                let stdin_fd = opt_int(args, 6).unwrap_or(DEVNULL);
                let stdout_fd = opt_int(args, 9).unwrap_or(DEVNULL);
                let stderr_fd = opt_int(args, 11).unwrap_or(DEVNULL);
                // `std::process::Command` takes OWNERSHIP of every fd it is
                // given and closes it in the parent after `spawn()`. CPython's
                // `fork_exec` closes nothing in the parent: the caller keeps
                // the ends it wrapped in file objects (`p2cwrite`, `c2pread`,
                // `errread`) and closes the rest itself in `_close_pipe_fds`.
                // Handing the raw fds over would therefore close the parent's
                // own pipe ends out from under its file objects — every read
                // then failed with EBADF — so each child-side fd is DUPLICATED
                // first and only the duplicate is given away.
                let mut wire =
                    |fd: i64, slot: fn(&mut std::process::Command, std::process::Stdio)| {
                        if fd >= 0 {
                            if let Some(dup) = dup_fd(fd as RawFd) {
                                slot(&mut command, std::process::Stdio::from(dup));
                            } else {
                                slot(&mut command, std::process::Stdio::null());
                            }
                        } else if fd == DEVNULL {
                            slot(&mut command, std::process::Stdio::null());
                        }
                    };
                wire(stdin_fd, |c, s| {
                    c.stdin(s);
                });
                wire(stdout_fd, |c, s| {
                    c.stdout(s);
                });
                wire(stderr_fd, |c, s| {
                    c.stderr(s);
                });
                // Nothing of the caller's may survive into the child: its own
                // copies of the child's pipe ends would keep the parent's read
                // side from ever seeing EOF (an unkillable hang), and the
                // errpipe write end would stop the parent's EOF read.
                // `pass_fds` (arg 3, `fds_to_keep`): those descriptors must
                // survive into the child, so their `FD_CLOEXEC` is cleared
                // there — std closes every other inherited fd.
                let fds_to_keep: Vec<RawFd> = match &*args[3].borrow() {
                    PyObject::List(items) | PyObject::Tuple(items) => items
                        .iter()
                        .filter_map(|item| item.as_i64())
                        .filter(|fd| *fd >= 0)
                        .map(|fd| fd as RawFd)
                        .collect(),
                    _ => Vec::new(),
                };
                if !fds_to_keep.is_empty() {
                    unsafe {
                        use std::os::unix::process::CommandExt;
                        command.pre_exec(move || {
                            for fd in fds_to_keep.iter() {
                                crate::modules::os_fd_clear_cloexec(*fd);
                            }
                            Ok(())
                        });
                    }
                }

                let parent_side = [
                    opt_int(args, 6).unwrap_or(-1) as RawFd,  // p2cread
                    opt_int(args, 8).unwrap_or(-1) as RawFd,  // c2pread
                    opt_int(args, 10).unwrap_or(-1) as RawFd, // errread
                    opt_int(args, 7).unwrap_or(-1) as RawFd,  // p2cwrite
                    opt_int(args, 9).unwrap_or(-1) as RawFd,  // c2pwrite
                    opt_int(args, 11).unwrap_or(-1) as RawFd, // errwrite
                    errpipe_write,
                ];
                unsafe {
                    use std::os::unix::process::CommandExt;
                    command.pre_exec(move || {
                        for fd in parent_side {
                            if fd >= 0 {
                                drop(std::fs::File::from_raw_fd(fd));
                            }
                        }
                        Ok(())
                    });
                }

                match command.spawn() {
                    Ok(child) => {
                        let pid = child.id() as i64;
                        // Keep the `Child` handle for the process's lifetime:
                        // dropping it does NOT reap (std only reaps inside
                        // `wait()`), and `Popen` reaps through `os.waitpid`,
                        // exactly as CPython does — the handle is kept so
                        // `os.waitpid` can service that call and nothing
                        // reaps the child behind `Popen`'s back.
                        SPAWNED_CHILDREN.with(|c| {
                            c.borrow_mut().insert(pid, child);
                        });
                        // CPython's `fork_exec` returns the pid and nothing
                        // else: `subprocess.py` assigns it straight to
                        // `self.pid`, and a tuple here made every later
                        // `self.pid` use (including `__del__`'s
                        // ResourceWarning message) misbehave.
                        Ok(py_int(pid))
                    }
                    Err(e) => {
                        if errpipe_write >= 0 {
                            write_exec_error(
                                errpipe_write,
                                "OSError",
                                &format!("{:x}", e.raw_os_error().unwrap_or(2)),
                                &e.to_string(),
                            );
                        }
                        // No child exists; `subprocess.py` reports the
                        // exec failure from the errpipe it just read.
                        Ok(py_int(-1))
                    }
                }
            },
        }),
    );
    d.insert_str(
        "close_fds",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "close_fds".to_string(),
            func: |_| Ok(py_none()),
        }),
    );
    d
}

/// `close(2)` without pulling in a new dependency: `File::from_raw_fd` +
/// drop is the same syscall, and this is the only close we need.
fn nix_close(fd: RawFd) -> std::io::Result<()> {
    use std::os::fd::FromRawFd;
    let file = unsafe { std::fs::File::from_raw_fd(fd) };
    drop(file);
    Ok(())
}

/// Wait for a child this interpreter spawned and return its `waitpid(2)`
/// status word (exit code in the high byte, terminating signal in the low
/// 7 bits — the encoding `os.WEXITSTATUS`/`WTERMSIG` decode).
///
/// This is how `os.waitpid` is implemented here: there is no libc binding
/// available for the raw syscall, but every child that can be waited for is
/// one THIS interpreter spawned (and therefore has a live `Child` handle in
/// `SPAWNED_CHILDREN`). A pid we do not own is reported as ECHILD, which is
/// what CPython does for a non-child too.
pub fn wait_for_child(pid: i64) -> Result<i32, i32> {
    let outcome = SPAWNED_CHILDREN.with(|c| {
        let mut children = c.borrow_mut();
        let child = match children.get_mut(&pid) {
            Some(child) => child,
            None => return Err(ECHILD),
        };
        match child.wait() {
            Ok(status) => {
                children.remove(&pid);
                Ok(status_word(&status))
            }
            Err(e) => Err(e.raw_os_error().unwrap_or(ECHILD)),
        }
    });
    outcome
}

/// Reap a child without blocking when it has already exited (the
/// `os.WNOHANG` case).
pub fn try_wait_child(pid: i64) -> Result<Option<i32>, i32> {
    SPAWNED_CHILDREN.with(|c| {
        let mut children = c.borrow_mut();
        let child = match children.get_mut(&pid) {
            Some(child) => child,
            None => return Err(ECHILD),
        };
        match child.try_wait() {
            Ok(Some(status)) => {
                children.remove(&pid);
                Ok(Some(status_word(&status)))
            }
            Ok(None) => Ok(None),
            Err(e) => Err(e.raw_os_error().unwrap_or(ECHILD)),
        }
    })
}

pub const ECHILD: i32 = 10;

/// `os.WNOHANG` as CPython defines it.
pub const WNOHANG_CONST: i64 = 1;

#[cfg(unix)]
fn status_word(status: &std::process::ExitStatus) -> i32 {
    use std::os::unix::process::ExitStatusExt;
    if let Some(code) = status.code() {
        (code & 0xff) << 8
    } else {
        // Terminated by a signal: the low 7 bits carry it (and bit 7 marks
        // "signalled", exactly as the kernel's waitpid status word does).
        let signal = status.signal().unwrap_or(0);
        signal & 0x7f
    }
}

/// Is this pid one of ours (i.e. could `waitpid` ever succeed for it)?
pub fn is_our_child(pid: i64) -> bool {
    SPAWNED_CHILDREN.with(|c| c.borrow().contains_key(&pid))
}

/// `fork_exec`'s `env_list`: CPython builds it as a list of
/// `b'KEY=VALUE'` byte strings (see `Lib/subprocess.py::_execute_child`), so
/// those are split here; `[key, value]` pairs are accepted too.
fn env_pairs(env: &PyObjectRef) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let items = match &*env.borrow() {
        PyObject::None => return out,
        PyObject::List(list) => list.clone(),
        PyObject::Tuple(tuple) => tuple.clone(),
        _ => return out,
    };
    for item in items {
        match &*item.borrow() {
            PyObject::Bytes(bytes) => {
                if let Some(idx) = bytes.iter().position(|b| *b == b'=') {
                    out.push((
                        String::from_utf8_lossy(&bytes[..idx]).into_owned(),
                        String::from_utf8_lossy(&bytes[idx + 1..]).into_owned(),
                    ));
                }
            }
            PyObject::Str(_) => {
                let text = to_py_string(&item);
                if let Some((key, value)) = text.split_once('=') {
                    out.push((key.to_string(), value.to_string()));
                }
            }
            PyObject::List(_) | PyObject::Tuple(_) => {
                let pair = match &*item.borrow() {
                    PyObject::List(list) => list.clone(),
                    PyObject::Tuple(tuple) => tuple.clone(),
                    _ => continue,
                };
                if pair.len() == 2 {
                    out.push((to_py_string(&pair[0]), to_py_string(&pair[1])));
                }
            }
            _ => {}
        }
    }
    out
}

fn to_py_string(obj: &PyObjectRef) -> String {
    match &*obj.borrow() {
        PyObject::Str(s) => s.to_string(),
        PyObject::None => String::new(),
        other => other.repr(),
    }
}

/// Duplicate a raw fd (CLOEXEC) without taking ownership of the original.
fn dup_fd(fd: RawFd) -> Option<std::fs::File> {
    // `ManuallyDrop` so the borrowed `File` never closes the fd we do not own.
    let borrowed = std::mem::ManuallyDrop::new(unsafe { std::fs::File::from_raw_fd(fd) });
    borrowed.try_clone().ok()
}

/// `setsid(2)` without a libc binding: a raw syscall, issued from a
/// `pre_exec` closure where only async-signal-safe calls are allowed.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
unsafe fn raw_setsid() {
    const SYS_SETSID: i64 = 112;
    let ret: i64;
    std::arch::asm!(
        "syscall",
        inlateout("rax") SYS_SETSID => ret,
        in("rdi") 0i64,
        lateout("rcx") _,
        lateout("r11") _,
        options(nostack)
    );
    let _ = ret;
}

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
unsafe fn raw_setsid() {}

/// `setpgid(0, 0)` — a raw syscall (x86_64 Linux number 106).
pub fn raw_setpgid_self() {
    raw_call2(106, 0, 0);
}

pub fn raw_call2(syscall: i64, a: i32, b: i32) -> i32 {
    raw_call3(syscall, a as i64, b as i64)
}

/// `getsid(2)` / `getpgid(2)` without a libc binding, via raw syscalls
/// (x86_64 Linux numbers: 124 / 121).
pub fn get_session_id(pid: i64) -> i32 {
    raw_call(124, pid as i32)
}

pub fn get_group_id(pid: i64) -> i32 {
    raw_call(121, pid as i32)
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn raw_call(syscall: i64, arg: i32) -> i32 {
    let ret: i64;
    // SAFETY: a plain syscall with a single scalar argument; `getpid`-style
    // calls have no memory effects.
    unsafe {
        std::arch::asm!(
            "syscall",
            inlateout("rax") syscall => ret,
            in("rdi") arg as i64,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack)
        );
    }
    ret as i32
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn raw_call3(syscall: i64, a: i64, b: i64) -> i32 {
    let ret: i64;
    // SAFETY: a plain two-argument syscall; `setpgid`/`getsid` have no
    // memory effects.
    unsafe {
        std::arch::asm!(
            "syscall",
            inlateout("rax") syscall => ret,
            in("rdi") a,
            in("rsi") b,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack)
        );
    }
    ret as i32
}

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
fn raw_call3(_syscall: i64, _a: i64, _b: i64) -> i32 {
    -1
}

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
fn raw_call(_syscall: i64, _arg: i32) -> i32 {
    -1
}

/// `getpid(2)` (x86_64 Linux syscall 39) — used by `os.abort`.
pub fn self_pid() -> i32 {
    raw_call(39, 0)
}
