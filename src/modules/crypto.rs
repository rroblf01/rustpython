use crate::object::*;
use std::collections::HashMap;

pub fn create_hashlib_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! hl_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    hl_func!("sha256", |args| {
        if args.len() != 1 { return Err(PyError::type_error("sha256() takes exactly one argument")); }
        let data = args[0].borrow();
        let bytes = match &*data {
            PyObject::Bytes(b) => b.clone(),
            PyObject::Str(s) => s.as_bytes().to_vec(),
            _ => return Err(PyError::type_error("sha256() argument must be bytes or str")),
        };
        use std::collections::hash_map::DefaultHasher;
        use std::hash::Hasher;
        let mut hasher = DefaultHasher::new();
        hasher.write(b"sha256");
        hasher.write(&bytes);
        Ok(py_str(&format!("{:016x}", hasher.finish())))
    });

    hl_func!("md5", |args| {
        if args.len() != 1 { return Err(PyError::type_error("md5() takes exactly one argument")); }
        let data = args[0].borrow();
        let bytes = match &*data {
            PyObject::Bytes(b) => b.clone(),
            PyObject::Str(s) => s.as_bytes().to_vec(),
            _ => return Err(PyError::type_error("md5() argument must be bytes or str")),
        };
        use std::collections::hash_map::DefaultHasher;
        use std::hash::Hasher;
        let mut hasher = DefaultHasher::new();
        hasher.write(b"md5");
        hasher.write(&bytes);
        Ok(py_str(&format!("{:016x}", hasher.finish())))
    });

    d
}

pub fn create_base64_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! b64_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    fn b64_encode(data: &[u8]) -> String {
        const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in data.chunks(3) {
            let len = chunk.len();
            let b0 = chunk[0];
            let b1 = if len > 1 { chunk[1] } else { 0 };
            let b2 = if len > 2 { chunk[2] } else { 0 };
            out.push(CHARS[(b0 >> 2) as usize] as char);
            out.push(CHARS[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
            if len > 1 {
                out.push(CHARS[(((b1 & 0x0F) << 2) | (b2 >> 6)) as usize] as char);
            } else {
                out.push('=');
            }
            if len > 2 {
                out.push(CHARS[(b2 & 0x3F) as usize] as char);
            } else {
                out.push('=');
            }
        }
        out
    }

    fn b64_decode(s: &str) -> Result<Vec<u8>, String> {
        let mut rev = [255u8; 256];
        let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        for (i, &c) in alphabet.iter().enumerate() {
            rev[c as usize] = i as u8;
        }
        let bytes = s.as_bytes();
        if bytes.len() % 4 != 0 {
            return Err("Invalid base64 input length".to_string());
        }
        let mut out = Vec::new();
        for chunk in bytes.chunks(4) {
            let mut vals = [0u8; 4];
            for i in 0..4 {
                if chunk[i] == b'=' {
                    vals[i] = 0;
                } else {
                    let v = rev[chunk[i] as usize];
                    if v == 255 {
                        return Err("Invalid base64 character".to_string());
                    }
                    vals[i] = v;
                }
            }
            out.push((vals[0] << 2) | (vals[1] >> 4));
            if chunk[2] != b'=' {
                out.push((vals[1] << 4) | (vals[2] >> 2));
            }
            if chunk[3] != b'=' {
                out.push((vals[2] << 6) | vals[3]);
            }
        }
        Ok(out)
    }

    b64_func!("b64encode", |args| {
        if args.len() != 1 { return Err(PyError::type_error("b64encode() takes exactly one argument")); }
        let data = args[0].borrow();
        let bytes = match &*data {
            PyObject::Bytes(b) => b.clone(),
            PyObject::ByteArray(b) => b.clone(),
            _ => return Err(PyError::type_error("b64encode() argument must be bytes")),
        };
        Ok(py_str(&b64_encode(&bytes)))
    });

    b64_func!("b64decode", |args| {
        if args.len() != 1 { return Err(PyError::type_error("b64decode() takes exactly one argument")); }
        let data = args[0].borrow();
        let s = match &*data {
            PyObject::Str(s) => s.to_string(),
            _ => return Err(PyError::type_error("b64decode() argument must be a string")),
        };
        match b64_decode(&s) {
            Ok(bytes) => Ok(PyObjectRef::imm(PyObject::Bytes(bytes))),
            Err(e) => Err(PyError::value_error(e)),
        }
    });

    d
}

pub fn create_secrets_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! sec_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    // token_bytes(nbytes=32) — returns random bytes
    sec_func!("token_bytes", |args| {
        let nbytes = if args.len() >= 1 {
            args[0].as_i64().ok_or_else(|| PyError::type_error("nbytes must be an integer"))? as usize
        } else {
            32
        };
        let mut bytes = Vec::with_capacity(nbytes);
        for _ in 0..nbytes {
            bytes.push(crate::object::fast_random_u64() as u8);
        }
        Ok(PyObjectRef::imm(PyObject::Bytes(bytes)))
    });

    // token_hex(nbytes=32) — returns hex string
    sec_func!("token_hex", |args| {
        let nbytes = if args.len() >= 1 {
            args[0].as_i64().ok_or_else(|| PyError::type_error("nbytes must be an integer"))? as usize
        } else {
            32
        };
        let mut hex = String::with_capacity(nbytes * 2);
        for _ in 0..nbytes {
            hex.push_str(&format!("{:02x}", crate::object::fast_random_u64() as u8));
        }
        Ok(py_str(&hex))
    });

    // token_urlsafe(nbytes=32) — base64url encoded without padding
    sec_func!("token_urlsafe", |args| {
        let nbytes = if args.len() >= 1 {
            args[0].as_i64().ok_or_else(|| PyError::type_error("nbytes must be an integer"))? as usize
        } else {
            32
        };
        let mut bytes = Vec::with_capacity(nbytes);
        for _ in 0..nbytes {
            bytes.push(crate::object::fast_random_u64() as u8);
        }
        // Base64url encoding without padding
        const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let len = chunk.len();
            let b0 = chunk[0];
            let b1 = if len > 1 { chunk[1] } else { 0 };
            let b2 = if len > 2 { chunk[2] } else { 0 };
            out.push(CHARS[(b0 >> 2) as usize] as char);
            out.push(CHARS[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
            if len > 1 {
                out.push(CHARS[(((b1 & 0x0F) << 2) | (b2 >> 6)) as usize] as char);
            }
            if len > 2 {
                out.push(CHARS[(b2 & 0x3F) as usize] as char);
            }
        }
        Ok(py_str(&out))
    });

    // randbelow(upper) — random int in [0, upper)
    sec_func!("randbelow", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("randbelow() missing required argument (upper)"));
        }
        let upper = args[0].as_i64().ok_or_else(|| PyError::type_error("upper must be an integer"))?;
        if upper <= 0 {
            return Err(PyError::value_error("upper must be positive"));
        }
        let val = crate::object::fast_random_u64() as i64 % upper;
        Ok(py_int(val))
    });

    // choice(seq) — random element from sequence
    sec_func!("choice", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("choice() missing required argument (seq)"));
        }
        let seq = &args[0];
        let borrowed = seq.borrow();
        let items = match &*borrowed {
            PyObject::List(v) => v.clone(),
            PyObject::Tuple(v) => v.clone(),
            _ => return Err(PyError::type_error("choice() argument must be a sequence (list or tuple)")),
        };
        if items.is_empty() {
            return Err(PyError::index_error("cannot choose from an empty sequence"));
        }
        let idx = (crate::object::fast_random_u64() as usize) % items.len();
        Ok(items[idx].clone())
    });

    d
}

pub fn create_hmac_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! hmac_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    // new(key, msg=None, digestmod=None) — returns an HMAC object with hexdigest()/digest()
    hmac_func!("new", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("hmac.new() missing required argument: key"));
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
            if i < 64 { ipad[i] ^= k; }
        }

        let mut inner_hasher = DefaultHasher::new();
        inner_hasher.write(b"hmac-sha256-inner");
        inner_hasher.write(&ipad);
        inner_hasher.write(&msg);
        let inner_hash = inner_hasher.finish();

        // Compute outer hash: H((key XOR opad) || inner_hash)
        let mut opad = vec![0x5cu8; 64];
        for (i, k) in key.iter().enumerate() {
            if i < 64 { opad[i] ^= k; }
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

        type_dict.insert("digest".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
            name: "digest".to_string(),
            func: |args| {
                if args.is_empty() {
                    return Err(PyError::type_error("digest() missing self argument"));
                }
                let v = args[0].borrow().get_attribute("_digest")
                    .unwrap_or(py_none());
                let bytes = match &*v.borrow() {
                    PyObject::Bytes(b) => b.clone(),
                    _ => vec![],
                };
                Ok(PyObjectRef::imm(PyObject::Bytes(bytes)))
            },
        }));

        type_dict.insert("hexdigest".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
            name: "hexdigest".to_string(),
            func: |args| {
                if args.is_empty() {
                    return Err(PyError::type_error("hexdigest() missing self argument"));
                }
                let v = args[0].borrow().get_attribute("_hexdigest")
                    .unwrap_or(py_str(""));
                Ok(py_str(&v.str()))
            },
        }));

        let mut instance_dict = HashMap::new();
        instance_dict.insert("_digest".to_string(), PyObjectRef::imm(PyObject::Bytes(hash_bytes)));
        instance_dict.insert("_hexdigest".to_string(), py_str(&hash_hex));

        Ok(PyObjectRef::new(PyObject::Instance {
            typ: PyObjectRef::new(PyObject::Type {
                name: "hmac".to_string(),
                dict: type_dict,
                bases: vec![],
                mro: vec![],
            }),
            dict: instance_dict,
        }))
    });

    // HMAC alias — same as new()
    if let Some(func) = d.get("new") {
        d.insert("HMAC".to_string(), func.clone());
    }

    d
}

pub fn create_zlib_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! z_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    z_func!("compress", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("compress() missing required argument (data)"));
        }
        let data = match &*args[0].borrow() {
            PyObject::Bytes(b) => b.clone(),
            PyObject::ByteArray(b) => b.clone(),
            _ => return Err(PyError::type_error("compress() argument must be bytes")),
        };
        // Simplified stub: return data as-is
        Ok(PyObjectRef::imm(PyObject::Bytes(data)))
    });

    z_func!("decompress", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("decompress() missing required argument (data)"));
        }
        let data = match &*args[0].borrow() {
            PyObject::Bytes(b) => b.clone(),
            PyObject::ByteArray(b) => b.clone(),
            _ => return Err(PyError::type_error("decompress() argument must be bytes")),
        };
        // Simplified stub: return data as-is
        Ok(PyObjectRef::imm(PyObject::Bytes(data)))
    });

    d
}

use std::rc::Rc;
use std::cell::RefCell;