use crate::object::*;
use std::collections::HashMap;

fn make_uuid(hex32: String) -> PyObjectRef {
    let mut type_dict = HashMap::new();

    type_dict.insert_str(
        "__str__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__str__".to_string(),
            func: |args| {
                if let PyObject::Instance { dict, .. } = &*args[0].borrow() {
                    if let Some(h) = dict.get_str("_hex") {
                        let s = h.str();
                        return Ok(py_str(&format!(
                            "{}-{}-{}-{}-{}",
                            &s[0..8],
                            &s[8..12],
                            &s[12..16],
                            &s[16..20],
                            &s[20..32]
                        )));
                    }
                }
                Err(PyError::runtime_error("UUID instance missing _hex"))
            },
        }),
    );
    type_dict.insert_str(
        "__repr__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__repr__".to_string(),
            func: |args| {
                if let PyObject::Instance { dict, .. } = &*args[0].borrow() {
                    if let Some(h) = dict.get_str("_hex") {
                        let s = h.str();
                        return Ok(py_str(&format!(
                            "UUID('{}-{}-{}-{}-{}')",
                            &s[0..8],
                            &s[8..12],
                            &s[12..16],
                            &s[16..20],
                            &s[20..32]
                        )));
                    }
                }
                Err(PyError::runtime_error("UUID instance missing _hex"))
            },
        }),
    );
    type_dict.insert_str(
        "__eq__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__eq__".to_string(),
            func: |args| {
                let self_hex = if let PyObject::Instance { dict, .. } = &*args[0].borrow() {
                    dict.get_str("_hex").map(|h| h.str())
                } else {
                    None
                };
                let other_hex = if let PyObject::Instance { dict, .. } = &*args[1].borrow() {
                    dict.get_str("_hex").map(|h| h.str())
                } else {
                    None
                };
                Ok(py_bool(self_hex.is_some() && self_hex == other_hex))
            },
        }),
    );
    type_dict.insert_str(
        "__hash__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__hash__".to_string(),
            func: |args| {
                if let PyObject::Instance { dict, .. } = &*args[0].borrow() {
                    if let Some(h) = dict.get_str("_hex") {
                        return builtin_hash(&[py_str(&h.str())]);
                    }
                }
                Err(PyError::runtime_error("UUID instance missing _hex"))
            },
        }),
    );
    let hex_getter = PyObjectRef::new(PyObject::BuiltinFunction {
        name: "hex".to_string(),
        func: |args| {
            if let PyObject::Instance { dict, .. } = &*args[0].borrow() {
                if let Some(h) = dict.get_str("_hex") {
                    return Ok(h.clone());
                }
            }
            Err(PyError::runtime_error("UUID instance missing _hex"))
        },
    });
    type_dict.insert_str(
        "hex",
        PyObjectRef::new(PyObject::Property(Box::new(PropertyData {
            getter: Some(hex_getter),
            setter: None,
            deleter: None,
            doc: None,
        }))),
    );
    let int_getter = PyObjectRef::new(PyObject::BuiltinFunction {
        name: "int".to_string(),
        func: |args| {
            if let PyObject::Instance { dict, .. } = &*args[0].borrow() {
                if let Some(h) = dict.get_str("_hex") {
                    let n = num_bigint::BigInt::parse_bytes(h.str().as_bytes(), 16)
                        .unwrap_or_else(|| num_bigint::BigInt::from(0));
                    return Ok(py_int(n));
                }
            }
            Err(PyError::runtime_error("UUID instance missing _hex"))
        },
    });
    type_dict.insert_str(
        "int",
        PyObjectRef::new(PyObject::Property(Box::new(PropertyData {
            getter: Some(int_getter),
            setter: None,
            deleter: None,
            doc: None,
        }))),
    );

    let typ = PyObjectRef::new(PyObject::Type {
        name: "UUID".to_string(),
        dict: Box::new(str_map_to_typedict(type_dict)),
        bases: vec![],
        mro: vec![],
    });
    PyObjectRef::new(PyObject::Instance {
        typ,
        dict: AttrMap::from([("_hex".to_string(), py_str(&hex32))]),
    })
}

fn random_uuid_hex(version: u8) -> String {
    let r1 = fast_random_u64();
    let r2 = fast_random_u64();
    let time_low = r1 as u32;
    let time_mid = (r1 >> 32) as u16;
    let time_hi_and_version = ((r1 >> 48) as u16 & 0x0FFF) | ((version as u16) << 12);
    let clock_seq = (r2 as u16 & 0x3FFF) | 0x8000;
    let node = (r2 >> 16) as u64;
    format!(
        "{:08x}{:04x}{:04x}{:04x}{:012x}",
        time_low, time_mid, time_hi_and_version, clock_seq, node
    )
}

pub fn create_uuid_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! uuid_func {
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

    uuid_func!("uuid4", |args| {
        if !args.is_empty() {
            return Err(PyError::type_error("uuid4() takes no arguments"));
        }
        Ok(make_uuid(random_uuid_hex(4)))
    });

    uuid_func!("uuid1", |_args| { Ok(make_uuid(random_uuid_hex(1))) });

    // uuid._ifconfig_getnode — get MAC address via ifconfig (Unix).
    // CPython's Lib/uuid.py calls this to obtain the hardware address.
    uuid_func!("_ifconfig_getnode", |_args| {
        // Try to read MAC from /sys/class/net/*/address (Linux) or
        // parse `ifconfig` output. In this single-process interpreter
        // we fall back to a random address if the real lookup fails.
        if let Ok(entries) = std::fs::read_dir("/sys/class/net") {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                if name_str == "lo" {
                    continue;
                }
                let addr_path = format!("/sys/class/net/{}/address", name_str);
                if let Ok(mac) = std::fs::read_to_string(&addr_path) {
                    let mac = mac.trim().replace(':', "");
                    if mac.len() == 12 && mac.chars().all(|c| c.is_ascii_hexdigit()) {
                        return Ok(py_int(i64::from_str_radix(&mac, 16).unwrap_or(0)));
                    }
                }
            }
        }
        // Fallback: random MAC
        Ok(py_int(
            i64::from_str_radix(&random_uuid_hex(0)[..12], 16).unwrap_or(0),
        ))
    });

    // UUID(hex=None, int=None, bytes=None) — supports the common construction forms.
    uuid_func!("UUID", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("UUID() missing required argument"));
        }
        let hex_arg = args[0].str();
        let cleaned: String = hex_arg.chars().filter(|c| c.is_ascii_hexdigit()).collect();
        if cleaned.len() != 32 {
            return Err(PyError::value_error("badly formed hexadecimal UUID string"));
        }
        Ok(make_uuid(cleaned.to_lowercase()))
    });

    d
}
