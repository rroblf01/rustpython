use crate::object::*;
use std::collections::HashMap;
use std::sync::atomic::AtomicI64;
use num_traits::{Signed, ToPrimitive};
use std::rc::Rc;

pub(crate) fn read_fd(fd: i32, buf: &mut Vec<u8>) -> std::io::Result<usize> {
    use std::io::Read;
    use std::os::unix::io::FromRawFd;
    // SAFETY: from_raw_fd takes ownership, but we use forget() to return it.
    let mut f = unsafe { std::fs::File::from_raw_fd(fd) };
    let result = f.read(buf);
    std::mem::forget(f); // Don't close the fd — caller still owns it
    result
}

/// Write to a raw file descriptor without taking ownership of the fd.
pub(crate) fn write_fd(fd: i32, data: &[u8]) -> std::io::Result<usize> {
    use std::io::Write;
    use std::os::unix::io::FromRawFd;
    // SAFETY: from_raw_fd takes ownership, but we use forget() to return it.
    let mut f = unsafe { std::fs::File::from_raw_fd(fd) };
    let result = f.write(data);
    std::mem::forget(f);
    result
}

/// Seek on a raw file descriptor (backs `os.lseek(fd, offset, whence)`).
/// Returns the resulting absolute offset.
pub(crate) fn lseek_fd(fd: i32, offset: i64, whence: i32) -> std::io::Result<i64> {
    use std::io::Seek;
    use std::io::SeekFrom;
    use std::os::unix::io::FromRawFd;
    // SAFETY: from_raw_fd takes ownership, but we use forget() to return it.
    let mut f = unsafe { std::fs::File::from_raw_fd(fd) };
    let seek_from = match whence {
        0 if offset >= 0 => SeekFrom::Start(offset as u64),
        0 => {
            std::mem::forget(f);
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "invalid argument",
            ));
        }
        1 => SeekFrom::Current(offset),
        2 => SeekFrom::End(offset),
        _ => {
            std::mem::forget(f);
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "invalid whence",
            ));
        }
    };
    let result = f.seek(seek_from);
    std::mem::forget(f);
    result.map(|pos| pos as i64)
}

/// Close a raw file descriptor by wrapping it in a File and dropping it.
pub(crate) fn close_fd(fd: i32) {
    use std::os::unix::io::FromRawFd;
    // SAFETY: from_raw_fd takes ownership; dropping it below closes the fd.
    let file = unsafe { std::fs::File::from_raw_fd(fd) };
    drop(file); // Closes the fd
}

/// Get an independently-owned, safely-droppable `File` for a standard
/// stream fd (0/1/2) without ever opening `/dev/stdout`-style paths: doing
/// so via `File::create` implies `O_TRUNC`, and every `VirtualMachine::new()`
/// (including the disposable, throwaway VMs Rust builtins spin up to invoke
/// a Python-level method — see `call_bound_method`) rebuilds `sys.stdout`,
/// truncating the *real* process stdout out from under any output already
/// written by the outer, real VM. `try_clone()` duplicates the fd instead
/// (like `dup()`), sharing the real stream's file offset without truncating
/// it and without risking the real fd getting closed when this VM drops.
pub(crate) fn dup_std_fd(fd: i32) -> std::io::Result<std::fs::File> {
    use std::os::unix::io::FromRawFd;
    // SAFETY: from_raw_fd takes ownership, but we forget() it right after
    // cloning so the real fd (0/1/2) is never closed by this wrapper.
    let borrowed = unsafe { std::fs::File::from_raw_fd(fd) };
    let dup = borrowed.try_clone();
    std::mem::forget(borrowed);
    dup
}


pub fn os_kill_builtin(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Err(PyError::type_error("kill() takes exactly 2 arguments"));
    }
    let pid = args[0]
        .as_i64()
        .ok_or_else(|| PyError::type_error("pid must be an int"))?;
    let signum = args[1]
        .as_i64()
        .ok_or_else(|| PyError::type_error("sig must be an int"))?;
    if pid == std::process::id() as i64 {
        crate::object::with_vm_mut(|vm| crate::modules::invoke_signal_handler_impl(vm, signum))??;
    }
    Ok(py_none())
}

// --- Helper: convert fs::Metadata to stat dict ---
pub(crate) fn stat_to_dict(meta: &std::fs::Metadata) -> HashMap<String, PyObjectRef> {
    use std::os::unix::fs::MetadataExt;
    let mut d = HashMap::new();
    d.insert_str("st_mode", py_int(meta.mode() as i64));
    d.insert_str("st_ino", py_int(meta.ino() as i64));
    d.insert_str("st_dev", py_int(meta.dev() as i64));
    d.insert_str("st_nlink", py_int(meta.nlink() as i64));
    d.insert_str("st_uid", py_int(meta.uid() as i64));
    d.insert_str("st_gid", py_int(meta.gid() as i64));
    d.insert_str("st_size", py_int(meta.size() as i64));
    if let Ok(t) = meta.modified() {
        let dur = t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        d.insert_str("st_mtime", py_float(dur.as_secs_f64()));
    }
    if let Ok(t) = meta.accessed() {
        let dur = t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        d.insert_str("st_atime", py_float(dur.as_secs_f64()));
    }
    if let Ok(t) = meta.created() {
        let dur = t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        d.insert_str("st_ctime", py_float(dur.as_secs_f64()));
    }
    d
}

// CPython's os functions raise `ValueError: embedded null character`
// when given a path containing a NUL byte (not an OSError) — the io
// layer's `InvalidInput` error must be translated accordingly (real
// trigger: `test_genericpath.py::test_invalid_paths`, which asserts
// `assertRaisesRegex(ValueError, 'embedded null')` for NUL paths, and
// `genericpath.exists`/`isfile`/`isdir`, which catch `(OSError,
// ValueError)` and must return False for such paths).
pub(crate) fn os_path_arg(obj: &PyObjectRef) -> Result<String, PyError> {
    let s = crate::object::path_arg_to_string(obj);
    if s.contains('\0') {
        return Err(PyError::value_error("embedded null character"));
    }
    Ok(s)
}

pub(crate) fn stat_dev_ino(meta: &std::fs::Metadata) -> (i64, i64) {
    use std::os::unix::fs::MetadataExt;
    (meta.ino() as i64, meta.dev() as i64)
}

// os.fstat(fd) / os.stat(int) — `std::fs::File::from_raw_fd` takes
// ownership of the fd, so forget it right after grabbing metadata to
// avoid closing a caller-owned descriptor.
pub(crate) fn fstat_result(fd: i64) -> PyResult<PyObjectRef> {
    use std::os::unix::io::FromRawFd;
    let file = unsafe { std::fs::File::from_raw_fd(fd as i32) };
    let res = file.metadata();
    std::mem::forget(file);
    match res {
        Ok(meta) => Ok(create_module("stat_result", stat_to_dict(&meta))),
        Err(e) => Err(PyError::os_error_from_io(&e)),
    }
}

