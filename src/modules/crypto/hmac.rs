use crate::object::*;
use std::collections::HashMap;

pub fn create_hmac_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! hmac_func {
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

    // `hmac.compare_digest` — CPython's own `test_hmac.py` asserts this IS
    // `_operator._compare_digest` (same object), so register the shared
    // instance (see `core::shared_compare_digest`).
    d.insert_str(
        "compare_digest",
        crate::modules::core::shared_compare_digest(),
    );

    // new(key, msg=None, digestmod=None) — returns an HMAC object with hexdigest()/digest()
    hmac_func!("new", |args| {
        if args.is_empty() {
            return Err(PyError::type_error(
                "hmac.new() missing required argument: key",
            ));
        }
        let key = match &*args[0].borrow() {
            PyObject::Bytes(b) => b.clone(),
            PyObject::Str(s) => s.as_bytes().to_vec(),
            _ => return Err(PyError::type_error("key must be bytes or str")),
        };
        let msg = if args.len() > 1 {
            match &*args[1].borrow() {
                PyObject::Bytes(b) => b.clone(),
                PyObject::Str(s) => s.as_bytes().to_vec(),
                _ => vec![],
            }
        } else {
            vec![]
        };

        // Build a combined hash using DefaultHasher (simplified HMAC)
        use std::collections::hash_map::DefaultHasher;
        use std::hash::Hasher;

        // Compute inner hash: H((key XOR ipad) || msg)
        let mut ipad = vec![0x36u8; 64];
        for (i, k) in key.iter().enumerate() {
            if i < 64 {
                ipad[i] ^= k;
            }
        }

        let mut inner_hasher = DefaultHasher::new();
        inner_hasher.write(b"hmac-sha256-inner");
        inner_hasher.write(&ipad);
        inner_hasher.write(&msg);
        let inner_hash = inner_hasher.finish();

        // Compute outer hash: H((key XOR opad) || inner_hash)
        let mut opad = vec![0x5cu8; 64];
        for (i, k) in key.iter().enumerate() {
            if i < 64 {
                opad[i] ^= k;
            }
        }

        let mut outer_hasher = DefaultHasher::new();
        outer_hasher.write(b"hmac-sha256-outer");
        outer_hasher.write(&opad);
        outer_hasher.write(&inner_hash.to_le_bytes());
        let outer_hash = outer_hasher.finish();

        let hash_bytes = outer_hash.to_le_bytes().to_vec();
        let hash_hex = format!("{:016x}", outer_hash);

        // Build hmac instance with hexdigest and digest methods
        // Store hash values in instance dict; methods read from self
        let mut type_dict = HashMap::new();

        type_dict.insert_str(
            "digest",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "digest".to_string(),
                func: |args| {
                    if args.is_empty() {
                        return Err(PyError::type_error("digest() missing self argument"));
                    }
                    let v = args[0]
                        .borrow()
                        .get_attribute("_digest")
                        .unwrap_or(py_none());
                    let bytes = match &*v.borrow() {
                        PyObject::Bytes(b) => b.clone(),
                        _ => vec![],
                    };
                    Ok(PyObjectRef::imm(PyObject::Bytes(bytes)))
                },
            }),
        );

        type_dict.insert_str(
            "hexdigest",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "hexdigest".to_string(),
                func: |args| {
                    if args.is_empty() {
                        return Err(PyError::type_error("hexdigest() missing self argument"));
                    }
                    let v = args[0]
                        .borrow()
                        .get_attribute("_hexdigest")
                        .unwrap_or(py_str(""));
                    Ok(py_str(&v.str()))
                },
            }),
        );

        let mut instance_dict = AttrMap::new();
        instance_dict.insert_str("_digest", PyObjectRef::imm(PyObject::Bytes(hash_bytes)));
        instance_dict.insert_str("_hexdigest", py_str(&hash_hex));

        Ok(PyObjectRef::new(PyObject::Instance {
            typ: PyObjectRef::new(PyObject::Type {
                name: "hmac".to_string(),
                dict: Box::new(str_map_to_typedict(type_dict)),
                bases: vec![],
                mro: vec![],
            }),
            dict: instance_dict,
        }))
    });

    // HMAC alias — same as new()
    if let Some(func) = d.get("new") {
        d.insert_str("HMAC", func.clone());
    }

    d
}
