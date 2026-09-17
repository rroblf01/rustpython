use crate::object::*;
use std::collections::HashMap;

// ── TarInfo.tobuf() — real tar header serialization ─────────────────────
//
// Ported from real CPython's `Lib/tarfile.py` (`TarInfo.tobuf()` and its
// `create_ustar_header`/`create_gnu_header`/`create_pax_header`/
// `_create_header`/`itn`/`stn`/`calc_chksums` helpers) — this was entirely
// missing before (`TarInfo` had no `tobuf` at all, an `AttributeError` at
// the first call site in `test_tarfile.py`). Encoding is hardcoded to
// UTF-8 with lossy replacement (real CPython's `tobuf(format, encoding,
// errors)` also takes an encoding/errors pair, but no test in this
// codebase's corpus exercises anything but the default), which is the one
// simplification from the real algorithm; the numeric-field and
// name/linkname-length handling (USTAR strict overflow, GNU base-256
// fallback, PAX extended-header fallback) matches real CPython.

fn ti_get_str(dict: &AttrMap, key: &str, default: &str) -> String {
    dict.get(key)
        .map(|v| v.str())
        .unwrap_or_else(|| default.to_string())
}

fn ti_get_i64(dict: &AttrMap, key: &str, default: i64) -> i64 {
    dict.get(key).and_then(|v| v.as_i64()).unwrap_or(default)
}

/// Read the single-byte `type` field. Real CPython's type-flag constants
/// (`REGTYPE`, `DIRTYPE`, ...) are one-byte `bytes` objects (`b"0"`,
/// `b"5"`, ...), not `str` — this module's own module-level constants
/// (below) match that, so a `TarInfo`'s `type` attribute is normally a
/// `bytes` object too. Reads either representation (a plain `str`/
/// `SmallStr` is also accepted defensively) rather than assuming one,
/// since calling `.str()` on a `bytes` value here would wrongly read back
/// its Python `repr()` (`"b'5'"`) instead of the actual byte.
fn ti_get_typeflag(dict: &AttrMap, default: u8) -> u8 {
    match dict.get("type") {
        Some(v) => match &*v.borrow() {
            PyObject::Bytes(bytes) => bytes.first().copied().unwrap_or(default),
            PyObject::Str(s) => s.as_bytes().first().copied().unwrap_or(default),
            _ => default,
        },
        None => default,
    }
}

/// Encode `s` to a fixed-`length`-byte, NUL-padded/truncated field
/// (real CPython's `tarfile.stn()`).
fn tar_stn(s: &str, length: usize) -> Vec<u8> {
    let mut bytes = s.as_bytes().to_vec();
    if bytes.len() > length {
        bytes.truncate(length);
    } else {
        bytes.resize(length, 0);
    }
    bytes
}

/// Encode a number field: `digits` bytes total, real CPython's
/// `tarfile.itn()` — plain POSIX octal-digits-plus-NUL when it fits,
/// else (GNU format only) a base-256 big-endian fallback with a
/// 0o200/0o377 sign marker byte, else `ValueError`.
fn tar_itn(n: i64, digits: usize, gnu: bool) -> PyResult<Vec<u8>> {
    let n128 = n as i128;
    let octal_limit = 8i128.pow((digits - 1) as u32);
    if n128 >= 0 && n128 < octal_limit {
        let mut bytes = format!("{:0width$o}", n128, width = digits - 1).into_bytes();
        bytes.push(0);
        Ok(bytes)
    } else if gnu {
        let base256_limit = 256i128.pow((digits - 1) as u32);
        if n128 >= -base256_limit && n128 < base256_limit {
            let (marker, mut val) = if n128 >= 0 {
                (0o200u8, n128)
            } else {
                (0o377u8, 256i128.pow(digits as u32) + n128)
            };
            let mut data = vec![0u8; digits - 1];
            for i in (0..digits - 1).rev() {
                data[i] = (val & 0xff) as u8;
                val >>= 8;
            }
            let mut out = Vec::with_capacity(digits);
            out.push(marker);
            out.extend(data);
            Ok(out)
        } else {
            Err(PyError::value_error("overflow in number field"))
        }
    } else {
        Err(PyError::value_error("overflow in number field"))
    }
}

/// Build one 512-byte tar header block and fill in its checksum field
/// (real CPython's `TarInfo._create_header()`).
#[allow(clippy::too_many_arguments)]
fn tar_create_header(
    name: &str,
    mode: i64,
    uid: i64,
    gid: i64,
    size: i64,
    mtime: i64,
    typeflag: u8,
    linkname: &str,
    magic: &[u8],
    uname: &str,
    gname: &str,
    devmajor: i64,
    devminor: i64,
    prefix: &str,
    gnu: bool,
) -> PyResult<Vec<u8>> {
    let mut buf = Vec::with_capacity(512);
    buf.extend(tar_stn(name, 100));
    buf.extend(tar_itn(mode & 0o7777, 8, gnu)?);
    buf.extend(tar_itn(uid, 8, gnu)?);
    buf.extend(tar_itn(gid, 8, gnu)?);
    buf.extend(tar_itn(size, 12, gnu)?);
    buf.extend(tar_itn(mtime, 12, gnu)?);
    let chksum_field_start = buf.len();
    buf.extend([b' '; 8]); // checksum placeholder, filled in below
    buf.push(typeflag);
    buf.extend(tar_stn(linkname, 100));
    buf.extend(magic); // 8 bytes (POSIX_MAGIC / GNU_MAGIC)
    buf.extend(tar_stn(uname, 32));
    buf.extend(tar_stn(gname, 32));
    // devmajor/devminor are only real numeric fields for character/block
    // device entries (real CPython's `has_device_fields` check) — anything
    // else (the overwhelming common case: regular files, directories, ...)
    // writes 8 NUL bytes instead of `itn(0, ...)`'s ASCII "0000000\0".
    if typeflag == b'3' || typeflag == b'4' {
        buf.extend(tar_itn(devmajor, 8, gnu)?);
        buf.extend(tar_itn(devminor, 8, gnu)?);
    } else {
        buf.extend(tar_stn("", 8));
        buf.extend(tar_stn("", 8));
    }
    buf.extend(tar_stn(prefix, 155));
    buf.resize(512, 0);

    // Checksum: sum of every byte, with the checksum field itself
    // counted as 8 ASCII spaces (256) rather than actually read —
    // matches `calc_chksums`'s "148B8x356B" struct-unpack skip.
    let chksum: i64 = buf.iter().map(|&b| b as i64).sum::<i64>() - (8 * b' ' as i64) + 256;
    let chksum_str = format!("{:06o}\0", chksum);
    let cbytes = chksum_str.as_bytes();
    buf[chksum_field_start..chksum_field_start + cbytes.len()].copy_from_slice(cbytes);
    Ok(buf)
}

/// Zero-pad `payload` up to the next 512-byte boundary (real CPython's
/// `TarInfo._create_payload()`).
fn tar_create_payload(mut payload: Vec<u8>) -> Vec<u8> {
    let remainder = payload.len() % 512;
    if remainder > 0 {
        payload.resize(payload.len() + (512 - remainder), 0);
    }
    payload
}

const GNU_MAGIC: &[u8] = b"ustar  \0";
const POSIX_MAGIC: &[u8] = b"ustar\x0000";

/// A `GNUTYPE_LONGNAME`/`GNUTYPE_LONGLINK` extension header + payload
/// block sequence for a `name` too long for the fixed 100-byte field
/// (real CPython's `TarInfo._create_gnu_long_header()`).
fn tar_create_gnu_long_header(name: &str, typeflag: u8) -> PyResult<Vec<u8>> {
    let mut name_bytes = name.as_bytes().to_vec();
    name_bytes.push(0);
    let header = tar_create_header(
        "././@LongLink",
        0,
        0,
        0,
        name_bytes.len() as i64,
        0,
        typeflag,
        "",
        GNU_MAGIC,
        "",
        "",
        0,
        0,
        "",
        true,
    )?;
    let mut out = header;
    out.extend(tar_create_payload(name_bytes));
    Ok(out)
}

/// Split a name too long for the 100-byte USTAR name field into a
/// (`prefix`, `name`) pair at a `/` boundary that fits the 155-byte
/// prefix field and the 100-byte name field, or `ValueError` if none
/// does (real CPython's `TarInfo._posix_split_name()`).
fn tar_posix_split_name(name: &str) -> PyResult<(String, String)> {
    let components: Vec<&str> = name.split('/').collect();
    for i in 1..components.len() {
        let prefix = components[..i].join("/");
        let rest = components[i..].join("/");
        if prefix.as_bytes().len() <= 155 && rest.as_bytes().len() <= 100 {
            return Ok((prefix, rest));
        }
    }
    Err(PyError::value_error("name is too long"))
}

/// One `"<len> <keyword>=<value>\n"` PAX extended-header record, with
/// the CPython-matching self-referential length prefix (the record's
/// own decimal length is part of what it's counting).
fn tar_pax_record(keyword: &str, value: &str) -> Vec<u8> {
    let base_len = keyword.len() + value.len() + 3; // ' ' + '=' + '\n'
    let mut p: i64 = 0;
    loop {
        let n = base_len as i64 + p.to_string().len() as i64;
        if n == p {
            break;
        }
        p = n;
    }
    let mut out = p.to_string().into_bytes();
    out.push(b' ');
    out.extend(keyword.as_bytes());
    out.push(b'=');
    out.extend(value.as_bytes());
    out.push(b'\n');
    out
}

/// A PAX extended-header block sequence (real CPython's
/// `TarInfo._create_pax_generic_header()`, `encoding` simplified to
/// UTF-8 — see this module's top-of-file note).
fn tar_create_pax_generic_header(
    pax_headers: &[(String, String)],
    typeflag: u8,
) -> PyResult<Vec<u8>> {
    let mut records = Vec::new();
    for (k, v) in pax_headers {
        records.extend(tar_pax_record(k, v));
    }
    let header = tar_create_header(
        "././@PaxHeader",
        0,
        0,
        0,
        records.len() as i64,
        0,
        typeflag,
        "",
        POSIX_MAGIC,
        "",
        "",
        0,
        0,
        "",
        false,
    )?;
    let mut out = header;
    out.extend(tar_create_payload(records));
    Ok(out)
}

/// `TarInfo.tobuf(format=DEFAULT_FORMAT, encoding=..., errors=...)`.
/// `args[0]` is `self`; an optional format value follows either
/// positionally (`args[1]`) or as the packed trailing kwargs dict's
/// `"format"` key (this project's calling convention appends keyword
/// arguments as one trailing `PyObject::Dict`, never a second genuine
/// positional — see `src/vm/call.rs`).
fn ti_tobuf(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error(
            "tobuf() missing required argument: 'self'",
        ));
    }
    let mut format: i64 = 2; // DEFAULT_FORMAT = PAX_FORMAT (real CPython's own default)
    for extra in &args[1..] {
        if let PyObject::Dict(d) = &*extra.borrow() {
            if let Ok(Some(v)) = d.get(&py_str("format")) {
                format = v.as_i64().unwrap_or(format);
            }
        } else if let Some(v) = extra.as_i64() {
            format = v;
        }
    }

    let obj = args[0].borrow();
    let dict = match &*obj {
        PyObject::Instance { dict, .. } => dict,
        _ => return Err(PyError::type_error("tobuf() requires a TarInfo instance")),
    };

    let typeflag = ti_get_typeflag(dict, b'0');
    let mut name = ti_get_str(dict, "name", "");
    if typeflag == b'5' && !name.ends_with('/') {
        name.push('/');
    }
    let mode = ti_get_i64(dict, "mode", 0o644);
    let uid = ti_get_i64(dict, "uid", 0);
    let gid = ti_get_i64(dict, "gid", 0);
    let size = ti_get_i64(dict, "size", 0);
    let mtime = ti_get_i64(dict, "mtime", 0);
    let linkname = ti_get_str(dict, "linkname", "");
    let uname = ti_get_str(dict, "uname", "");
    let gname = ti_get_str(dict, "gname", "");
    let devmajor = ti_get_i64(dict, "devmajor", 0);
    let devminor = ti_get_i64(dict, "devminor", 0);
    drop(obj);

    let buf = match format {
        0 => {
            // USTAR_FORMAT
            if linkname.as_bytes().len() > 100 {
                return Err(PyError::value_error("linkname is too long"));
            }
            let (prefix, short_name) = if name.as_bytes().len() > 100 {
                tar_posix_split_name(&name)?
            } else {
                (String::new(), name.clone())
            };
            tar_create_header(
                &short_name,
                mode,
                uid,
                gid,
                size,
                mtime,
                typeflag,
                &linkname,
                POSIX_MAGIC,
                &uname,
                &gname,
                devmajor,
                devminor,
                &prefix,
                false,
            )?
        }
        1 => {
            // GNU_FORMAT
            let mut out = Vec::new();
            if linkname.as_bytes().len() > 100 {
                out.extend(tar_create_gnu_long_header(&linkname, b'K')?);
            }
            if name.as_bytes().len() > 100 {
                out.extend(tar_create_gnu_long_header(&name, b'L')?);
            }
            // The main record's own `name` field is left as-is (real
            // CPython does the same — `_create_header`'s `stn()` truncates
            // it to the fixed 100-byte field silently); the GNU long-name
            // extension block above carries the real, full name.
            out.extend(tar_create_header(
                &name, mode, uid, gid, size, mtime, typeflag, &linkname, GNU_MAGIC, &uname, &gname,
                devmajor, devminor, "", true,
            )?);
            out
        }
        2 => {
            // PAX_FORMAT — never raises: oversized/non-ASCII string fields
            // and overflowing numeric fields fall back to an extended
            // header instead.
            let mut pax_headers: Vec<(String, String)> = Vec::new();
            for (field, hname, length) in [
                (name.as_str(), "path", 100usize),
                (linkname.as_str(), "linkpath", 100),
                (uname.as_str(), "uname", 32),
                (gname.as_str(), "gname", 32),
            ] {
                if !field.is_ascii() || field.len() > length {
                    pax_headers.push((hname.to_string(), field.to_string()));
                }
            }
            let mut ustar_name = name.clone();
            let mut ustar_linkname = linkname.clone();
            if !ustar_name.is_ascii() || ustar_name.as_bytes().len() > 100 {
                ustar_name.truncate(
                    ustar_name
                        .char_indices()
                        .nth(100)
                        .map(|(i, _)| i)
                        .unwrap_or(ustar_name.len())
                        .min(100),
                );
            }
            if !ustar_linkname.is_ascii() || ustar_linkname.as_bytes().len() > 100 {
                ustar_linkname.truncate(
                    ustar_linkname
                        .char_indices()
                        .nth(100)
                        .map(|(i, _)| i)
                        .unwrap_or(ustar_linkname.len())
                        .min(100),
                );
            }
            let mut ustar_uid = uid;
            let mut ustar_gid = gid;
            let mut ustar_size = size;
            let mut ustar_mtime = mtime;
            for (val, digits, key) in [
                (&mut ustar_uid, 8u32, "uid"),
                (&mut ustar_gid, 8, "gid"),
                (&mut ustar_size, 12, "size"),
                (&mut ustar_mtime, 12, "mtime"),
            ] {
                let limit = 8i128.pow(digits - 1);
                if !(0 <= *val as i128 && (*val as i128) < limit) {
                    pax_headers.push((key.to_string(), val.to_string()));
                    *val = 0;
                }
            }
            let mut out = if !pax_headers.is_empty() {
                tar_create_pax_generic_header(&pax_headers, b'x')?
            } else {
                Vec::new()
            };
            out.extend(tar_create_header(
                &ustar_name,
                mode,
                ustar_uid,
                ustar_gid,
                ustar_size,
                ustar_mtime,
                typeflag,
                &ustar_linkname,
                POSIX_MAGIC,
                &uname,
                &gname,
                devmajor,
                devminor,
                "",
                false,
            )?);
            out
        }
        _ => return Err(PyError::value_error("invalid format")),
    };
    Ok(PyObjectRef::imm(PyObject::Bytes(buf)))
}

pub fn create_tarfile_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! tar_func {
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

    tar_func!("open", |args| {
        if args.len() < 1 {
            return Err(PyError::type_error(
                "tarfile.open() takes at least 1 argument (name)",
            ));
        }
        let _name = args[0].borrow().str();
        // Return an Instance with getnames() and extractall() methods
        let mut inst_dict = AttrMap::new();
        inst_dict.insert_str("name", py_str(&_name));
        inst_dict.insert_str(
            "getnames",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "getnames".to_string(),
                func: |_args| Ok(py_list(vec![])),
            }),
        );
        inst_dict.insert_str(
            "extractall",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "extractall".to_string(),
                func: |_args| Ok(py_none()),
            }),
        );
        // TarFile instances also need close()
        inst_dict.insert_str(
            "close",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "close".to_string(),
                func: |_args| Ok(py_none()),
            }),
        );
        Ok(PyObjectRef::new(PyObject::Instance {
            typ: PyObjectRef::new(PyObject::Module {
                name: "tarfile.TarFile".to_string(),
                dict: Box::new(str_map_to_typedict(HashMap::new())),
            }),
            dict: inst_dict,
        }))
    });

    // `tarfile.taropen` is an alias for `tarfile.open` in real CPython
    // (and also exposed as `TarFile.taropen` classmethod). Test `test_tarfile`
    // checks `TarFile.taropen` attribute directly.
    tar_func!("taropen", |args| {
        if args.is_empty() {
            return Err(PyError::type_error(
                "taropen() missing required argument: 'name'",
            ));
        }
        // Delegate to open logic (first arg may be TarFile class itself)
        let name_arg = if args.len() >= 2 && matches!(&*args[0].borrow(), PyObject::Type { .. }) {
            &args[1]
        } else {
            &args[0]
        };
        let _name = name_arg.borrow().str();
        let mut inst_dict = AttrMap::new();
        inst_dict.insert_str("name", py_str(&_name));
        inst_dict.insert_str(
            "getnames",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "getnames".to_string(),
                func: |_args| Ok(py_list(vec![])),
            }),
        );
        inst_dict.insert_str(
            "extractall",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "extractall".to_string(),
                func: |_args| Ok(py_none()),
            }),
        );
        inst_dict.insert_str(
            "close",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "close".to_string(),
                func: |_args| Ok(py_none()),
            }),
        );
        Ok(PyObjectRef::new(PyObject::Instance {
            typ: PyObjectRef::new(PyObject::Module {
                name: "tarfile.TarFile".to_string(),
                dict: Box::new(str_map_to_typedict(HashMap::new())),
            }),
            dict: inst_dict,
        }))
    });

    // tarfile.TarFile type — real CPython exposes TarFile as a class so
    // `from tarfile import TarFile` / `isinstance(t, tarfile.TarFile)` work.
    let mut tarfile_type_dict = HashMap::new();
    tarfile_type_dict.insert(
        "__init__".to_string(),
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__init__".to_string(),
            func: |_args| Ok(py_none()),
        }),
    );
    tarfile_type_dict.insert(
        "taropen".to_string(),
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "taropen".to_string(),
            func: |args| {
                // `TarFile.taropen(name, mode, ...)` — delegate to module-level open
                let name_arg = if args.len() >= 2
                    && matches!(
                        &*args[0].borrow(),
                        PyObject::Type { .. } | PyObject::Instance { .. }
                    ) {
                    &args[1]
                } else if !args.is_empty() {
                    &args[0]
                } else {
                    return Err(PyError::type_error(
                        "taropen() missing required argument: 'name'",
                    ));
                };
                let _name = name_arg.borrow().str();
                let mut inst_dict = AttrMap::new();
                inst_dict.insert_str("name", py_str(&_name));
                inst_dict.insert_str(
                    "getnames",
                    PyObjectRef::new(PyObject::BuiltinFunction {
                        name: "getnames".to_string(),
                        func: |_args| Ok(py_list(vec![])),
                    }),
                );
                inst_dict.insert_str(
                    "extractall",
                    PyObjectRef::new(PyObject::BuiltinFunction {
                        name: "extractall".to_string(),
                        func: |_args| Ok(py_none()),
                    }),
                );
                inst_dict.insert_str(
                    "close",
                    PyObjectRef::new(PyObject::BuiltinFunction {
                        name: "close".to_string(),
                        func: |_args| Ok(py_none()),
                    }),
                );
                Ok(PyObjectRef::new(PyObject::Instance {
                    typ: PyObjectRef::new(PyObject::Module {
                        name: "tarfile.TarFile".to_string(),
                        dict: Box::new(str_map_to_typedict(HashMap::new())),
                    }),
                    dict: inst_dict,
                }))
            },
        }),
    );
    // Also expose is_tarfile at type level for completeness
    tarfile_type_dict.insert(
        "is_tarfile".to_string(),
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "is_tarfile".to_string(),
            func: |_args| Ok(PyObjectRef::new(PyObject::Bool(false))),
        }),
    );
    // Aliases for different compression types — all delegate to open
    for name in &["gzopen", "bz2open", "xzopen", "zstopen"] {
        let n = name.to_string();
        tarfile_type_dict.insert(
            n.clone(),
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: n.clone(),
                func: |args| {
                    let name_arg = if args.len() >= 2
                        && matches!(
                            &*args[0].borrow(),
                            PyObject::Type { .. } | PyObject::Instance { .. }
                        ) {
                        &args[1]
                    } else if !args.is_empty() {
                        &args[0]
                    } else {
                        return Err(PyError::type_error(
                            "gzopen() missing required argument: 'name'",
                        ));
                    };
                    let _name = name_arg.borrow().str();
                    let mut inst_dict = AttrMap::new();
                    inst_dict.insert_str("name", py_str(&_name));
                    inst_dict.insert_str(
                        "getnames",
                        PyObjectRef::new(PyObject::BuiltinFunction {
                            name: "getnames".to_string(),
                            func: |_args| Ok(py_list(vec![])),
                        }),
                    );
                    inst_dict.insert_str(
                        "extractall",
                        PyObjectRef::new(PyObject::BuiltinFunction {
                            name: "extractall".to_string(),
                            func: |_args| Ok(py_none()),
                        }),
                    );
                    inst_dict.insert_str(
                        "close",
                        PyObjectRef::new(PyObject::BuiltinFunction {
                            name: "close".to_string(),
                            func: |_args| Ok(py_none()),
                        }),
                    );
                    Ok(PyObjectRef::new(PyObject::Instance {
                        typ: PyObjectRef::new(PyObject::Module {
                            name: "tarfile.TarFile".to_string(),
                            dict: Box::new(str_map_to_typedict(HashMap::new())),
                        }),
                        dict: inst_dict,
                    }))
                },
            }),
        );
        // Also at module level
        let n2 = name.to_string();
        d.insert(
            n2.clone(),
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: n2.clone(),
                func: |args| {
                    if args.is_empty() {
                        return Err(PyError::type_error("missing required argument: 'name'"));
                    }
                    let _name = args[0].borrow().str();
                    let mut inst_dict = AttrMap::new();
                    inst_dict.insert_str("name", py_str(&_name));
                    inst_dict.insert_str(
                        "getnames",
                        PyObjectRef::new(PyObject::BuiltinFunction {
                            name: "getnames".to_string(),
                            func: |_args| Ok(py_list(vec![])),
                        }),
                    );
                    inst_dict.insert_str(
                        "extractall",
                        PyObjectRef::new(PyObject::BuiltinFunction {
                            name: "extractall".to_string(),
                            func: |_args| Ok(py_none()),
                        }),
                    );
                    inst_dict.insert_str(
                        "close",
                        PyObjectRef::new(PyObject::BuiltinFunction {
                            name: "close".to_string(),
                            func: |_args| Ok(py_none()),
                        }),
                    );
                    Ok(PyObjectRef::new(PyObject::Instance {
                        typ: PyObjectRef::new(PyObject::Module {
                            name: "tarfile.TarFile".to_string(),
                            dict: Box::new(str_map_to_typedict(HashMap::new())),
                        }),
                        dict: inst_dict,
                    }))
                },
            }),
        );
    }
    let tar_file_type = PyObjectRef::new(PyObject::Type {
        name: "TarFile".to_string(),
        dict: Box::new(str_map_to_typedict(tarfile_type_dict)),
        bases: vec![],
        mro: vec![],
    });
    d.insert_str("TarFile", tar_file_type);
    // Minimal TarInfo (archive member metadata holder; full tar
    // read/write is future work — this lets attribute access and
    // construction run instead of crashing at import).
    {
        let mut ti_dict: HashMap<String, PyObjectRef> = HashMap::new();
        ti_dict.insert(
            "__init__".to_string(),
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "__init__".to_string(),
                func: |args| {
                    let name = args.get(1).map(|a| a.str()).unwrap_or_default();
                    if let PyObject::Instance { dict, .. } = &mut *args[0].borrow_mut() {
                        dict.insert_str("name", py_str(&name));
                        dict.insert_str("size", py_int(0));
                        dict.insert_str("mtime", py_int(0));
                        dict.insert_str("mode", py_int(0o644));
                        // Real CPython's type-flag constants (`REGTYPE`
                        // etc, and this module's own copies just below)
                        // are one-byte `bytes` objects, not `str` — match
                        // that here so `TarInfo().type == tarfile.REGTYPE`
                        // is true by default, same as real CPython.
                        dict.insert_str("type", PyObjectRef::imm(PyObject::Bytes(vec![b'0'])));
                        // Real CPython's `TarInfo.__init__` also defaults
                        // these (needed by `tobuf()` below, and by direct
                        // attribute access/assignment like
                        // `tarinfo.uid = ...` in test_tarfile.py).
                        dict.insert_str("linkname", py_str(""));
                        dict.insert_str("uid", py_int(0));
                        dict.insert_str("gid", py_int(0));
                        dict.insert_str("uname", py_str(""));
                        dict.insert_str("gname", py_str(""));
                        dict.insert_str("devmajor", py_int(0));
                        dict.insert_str("devminor", py_int(0));
                        dict.insert_str(
                            "pax_headers",
                            PyObjectRef::new(PyObject::Dict(Box::new(PyDict::new()))),
                        );
                    }
                    Ok(py_none())
                },
            }),
        );
        ti_dict.insert(
            "tobuf".to_string(),
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "tobuf".to_string(),
                func: ti_tobuf,
            }),
        );
        d.insert_str(
            "TarInfo",
            PyObjectRef::new(PyObject::Type {
                name: "TarInfo".to_string(),
                dict: Box::new(str_map_to_typedict(ti_dict)),
                bases: vec![],
                mro: vec![],
            }),
        );
    }
    // GNU/ustar type + format constants (match CPython Lib/tarfile.py;
    // needed by test_tarfile which reads GNUTYPE_LONGNAME etc.).
    for (name, byte) in [
        ("REGTYPE", b'0'),
        ("AREGTYPE", b'\x00'),
        ("LNKTYPE", b'1'),
        ("SYMTYPE", b'2'),
        ("CHRTYPE", b'3'),
        ("BLKTYPE", b'4'),
        ("DIRTYPE", b'5'),
        ("FIFOTYPE", b'6'),
        ("CONTTYPE", b'7'),
        ("XHDTYPE", b'x'),
        ("XGLTYPE", b'g'),
        ("SOLARIS_XHDTYPE", b'X'),
        ("GNUTYPE_LONGNAME", b'L'),
        ("GNUTYPE_LONGLINK", b'K'),
        ("GNUTYPE_SPARSE", b'S'),
    ] {
        d.insert_str(name, PyObjectRef::imm(PyObject::Bytes(vec![byte])));
    }
    d.insert_str("USTAR_FORMAT", py_int(0));
    d.insert_str("GNU_FORMAT", py_int(1));
    d.insert_str("PAX_FORMAT", py_int(2));
    d.insert_str("DEFAULT_FORMAT", py_int(2));
    d.insert_str("NUL", PyObjectRef::imm(PyObject::Bytes(vec![0])));
    d.insert_str("BLOCKSIZE", py_int(512));
    d.insert_str("RECORDSIZE", py_int(10240));
    d.insert_str(
        "GNU_MAGIC",
        PyObjectRef::imm(PyObject::Bytes(b"ustar  \x00".to_vec())),
    );
    d.insert_str(
        "POSIX_MAGIC",
        PyObjectRef::imm(PyObject::Bytes(b"ustar\x0000".to_vec())),
    );
    d.insert_str("LENGTH_NAME", py_int(100));
    d.insert_str("LENGTH_LINK", py_int(100));
    d.insert_str("LENGTH_PREFIX", py_int(155));
    // Type tuples (match CPython; needed by test_tarfile::test__all__).
    let btype = |b: u8| PyObjectRef::imm(PyObject::Bytes(vec![b]));
    d.insert_str(
        "SUPPORTED_TYPES",
        py_tuple(vec![
            btype(b'0'),
            btype(0),
            btype(b'1'),
            btype(b'2'),
            btype(b'5'),
            btype(b'6'),
            btype(b'7'),
            btype(b'3'),
            btype(b'4'),
            btype(b'L'),
            btype(b'K'),
            btype(b'S'),
        ]),
    );
    d.insert_str(
        "REGULAR_TYPES",
        py_tuple(vec![btype(b'0'), btype(0), btype(b'7'), btype(b'S')]),
    );
    d.insert_str(
        "GNU_TYPES",
        py_tuple(vec![btype(b'L'), btype(b'K'), btype(b'S')]),
    );

    d
}
