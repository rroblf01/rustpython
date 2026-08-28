use crate::object::*;
use std::collections::HashMap;

// Moved here from object.rs (was under a "---- zipfile module ----" banner
// in the monolithic object.rs — see the file-splitting refactor's memory
// entry for context).
// Helper: extract ZIP entry data from an Instance's dict
fn zipfile_get_entry(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let self_obj = &args[0];
    let name = args[1].borrow().str();
    let (entries, data) = match &*self_obj.borrow() {
        PyObject::Instance { dict, .. } => {
            let entries = dict
                .get("_entries")
                .ok_or_else(|| {
                    PyError::runtime_error("ZipFile instance corrupted: missing _entries")
                })?
                .clone();
            let data = dict
                .get("_data")
                .ok_or_else(|| PyError::runtime_error("ZipFile instance corrupted: missing _data"))?
                .clone();
            (entries, data)
        }
        _ => {
            return Err(PyError::runtime_error(
                "ZipFile method called on non-instance",
            ))
        }
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
        if entry_list.len() < 5 {
            continue;
        }
        let entry_name = entry_list[0].borrow().str();
        if entry_name != name {
            continue;
        }
        let data_offset = match entry_list[1].as_i64() {
            Some(n) => n as usize,
            None => continue,
        };
        let compressed_size = match entry_list[2].as_i64() {
            Some(n) => n as usize,
            None => continue,
        };
        if data_offset + compressed_size > data_bytes.len() {
            return Err(PyError::runtime_error("ZipFile: data truncated in archive"));
        }
        let raw = data_bytes[data_offset..data_offset + compressed_size].to_vec();
        return Ok(PyObjectRef::new(PyObject::Bytes(raw)));
    }

    Err(PyError::key_error(format!(
        "File not found in zip: '{}'",
        name
    )))
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
            Err(PyError::runtime_error(
                "ZipFile instance corrupted: missing _names",
            ))
        }
        _ => Err(PyError::runtime_error("namelist() called on non-instance")),
    }
}

fn zipfile_read(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Err(PyError::type_error(
            "read() takes exactly one argument (name)",
        ));
    }
    zipfile_get_entry(args)
}

fn zipfile_extract(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Err(PyError::type_error(
            "extract() takes exactly one argument (name)",
        ));
    }
    zipfile_get_entry(args)
}

fn zipfile_infolist(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let self_obj = &args[0];
    let entries = match &*self_obj.borrow() {
        PyObject::Instance { dict, .. } => dict
            .get("_entries")
            .ok_or_else(|| PyError::runtime_error("ZipFile instance corrupted"))?
            .clone(),
        _ => return Err(PyError::runtime_error("infolist() called on non-instance")),
    };

    let entries_list = match &*entries.borrow() {
        PyObject::List(items) => items.clone(),
        _ => return Err(PyError::runtime_error("ZipFile entries corrupted")),
    };

    let infos: Vec<PyObjectRef> = entries_list
        .iter()
        .map(|entry| {
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
        })
        .collect();

    Ok(py_list(infos))
}

pub fn zipfile_constructor(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 1 || args.len() > 2 {
        return Err(PyError::type_error(
            "ZipFile() takes 1-2 arguments (filename, [mode])",
        ));
    }
    let filename = args[0].borrow().str();
    let mode = if args.len() > 1 {
        args[1].borrow().str()
    } else {
        "r".to_string()
    };
    if mode != "r" {
        return Err(PyError::value_error("ZipFile only supports mode='r'"));
    }

    // Read entire file into memory
    let archive = match std::fs::read(&filename) {
        Ok(d) => d,
        Err(e) => {
            return Err(PyError::runtime_error(format!(
                "Cannot open zip file '{}': {}",
                filename, e
            )))
        }
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
            archive[offset + 18],
            archive[offset + 19],
            archive[offset + 20],
            archive[offset + 21],
        ]) as usize;
        let uncompressed_size = u32::from_le_bytes([
            archive[offset + 22],
            archive[offset + 23],
            archive[offset + 24],
            archive[offset + 25],
        ]) as usize;
        let filename_length =
            u16::from_le_bytes([archive[offset + 26], archive[offset + 27]]) as usize;
        let extra_field_length =
            u16::from_le_bytes([archive[offset + 28], archive[offset + 29]]) as usize;

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
    inst_dict.insert(
        "_data".to_string(),
        PyObjectRef::new(PyObject::Bytes(archive)),
    );
    inst_dict.insert("_names".to_string(), py_list(names));
    inst_dict.insert("_entries".to_string(), py_list(entries));

    // Attach methods as BuiltinFunctions (will be wrapped as BuiltinMethod with self_obj)
    inst_dict.insert(
        "namelist".to_string(),
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "namelist".to_string(),
            func: zipfile_namelist,
        }),
    );
    inst_dict.insert(
        "read".to_string(),
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "read".to_string(),
            func: zipfile_read,
        }),
    );
    inst_dict.insert(
        "extract".to_string(),
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "extract".to_string(),
            func: zipfile_extract,
        }),
    );
    inst_dict.insert(
        "infolist".to_string(),
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "infolist".to_string(),
            func: zipfile_infolist,
        }),
    );

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
    d.insert_str(
        "ZipFile",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "ZipFile".to_string(),
            func: zipfile_constructor,
        }),
    );
    d
}
