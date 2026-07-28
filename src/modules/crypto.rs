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
        use sha2::Digest;
        let hash = sha2::Sha256::digest(&bytes);
        Ok(PyObjectRef::imm(PyObject::Bytes(hash.to_vec())))
    });

    // sha224/sha384/sha512 — same family as sha256, just missing. Added all
    // three together since they're the same trivial one-liner pattern and
    // real code reaches for any of them (Django's own `hashlib.sha224` via
    // `from hashlib import sha224` — real code in `django/db/backends/
    // sqlite3/base.py`'s `_sqlite_...` helpers` chain, plus general hashlib
    // completeness).
    hl_func!("sha224", |args| {
        if args.len() != 1 { return Err(PyError::type_error("sha224() takes exactly one argument")); }
        let data = args[0].borrow();
        let bytes = match &*data {
            PyObject::Bytes(b) => b.clone(),
            PyObject::Str(s) => s.as_bytes().to_vec(),
            _ => return Err(PyError::type_error("sha224() argument must be bytes or str")),
        };
        use sha2::Digest;
        let hash = sha2::Sha224::digest(&bytes);
        Ok(PyObjectRef::imm(PyObject::Bytes(hash.to_vec())))
    });

    hl_func!("sha384", |args| {
        if args.len() != 1 { return Err(PyError::type_error("sha384() takes exactly one argument")); }
        let data = args[0].borrow();
        let bytes = match &*data {
            PyObject::Bytes(b) => b.clone(),
            PyObject::Str(s) => s.as_bytes().to_vec(),
            _ => return Err(PyError::type_error("sha384() argument must be bytes or str")),
        };
        use sha2::Digest;
        let hash = sha2::Sha384::digest(&bytes);
        Ok(PyObjectRef::imm(PyObject::Bytes(hash.to_vec())))
    });

    hl_func!("sha512", |args| {
        if args.len() != 1 { return Err(PyError::type_error("sha512() takes exactly one argument")); }
        let data = args[0].borrow();
        let bytes = match &*data {
            PyObject::Bytes(b) => b.clone(),
            PyObject::Str(s) => s.as_bytes().to_vec(),
            _ => return Err(PyError::type_error("sha512() argument must be bytes or str")),
        };
        use sha2::Digest;
        let hash = sha2::Sha512::digest(&bytes);
        Ok(PyObjectRef::imm(PyObject::Bytes(hash.to_vec())))
    });

    hl_func!("sha1", |args| {
        if args.len() != 1 { return Err(PyError::type_error("sha1() takes exactly one argument")); }
        let data = args[0].borrow();
        let bytes = match &*data {
            PyObject::Bytes(b) => b.clone(),
            PyObject::Str(s) => s.as_bytes().to_vec(),
            _ => return Err(PyError::type_error("sha1() argument must be bytes or str")),
        };
        use sha1::Digest;
        let hash = sha1::Sha1::digest(&bytes);
        Ok(PyObjectRef::imm(PyObject::Bytes(hash.to_vec())))
    });

    hl_func!("md5", |args| {
        if args.len() != 1 { return Err(PyError::type_error("md5() takes exactly one argument")); }
        let data = args[0].borrow();
        let bytes = match &*data {
            PyObject::Bytes(b) => b.clone(),
            PyObject::Str(s) => s.as_bytes().to_vec(),
            _ => return Err(PyError::type_error("md5() argument must be bytes or str")),
        };
        use sha2::digest::Digest;
        let hash = md5::Md5::digest(&bytes);
        Ok(PyObjectRef::imm(PyObject::Bytes(hash.to_vec())))
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

    // CRITICAL, general — `b64encode` (and this file's other `bNNencode`
    // functions) returned a plain `str`, not `bytes` — real CPython's
    // `base64.b64encode`/etc. ALWAYS return `bytes` (encoded data is
    // fundamentally binary-safe output, not text), a distinction real code
    // relies on constantly: `base64.b64encode(data).decode('ascii')` (an
    // extremely common idiom to get a STR from the bytes result) raised
    // `AttributeError: 'str' object has no attribute 'decode'` here, since
    // there was already a `str` where a real `bytes` was expected.
    b64_func!("b64encode", |args| {
        if args.len() != 1 { return Err(PyError::type_error("b64encode() takes exactly one argument")); }
        let data = args[0].borrow();
        let bytes = match &*data {
            PyObject::Bytes(b) => b.clone(),
            PyObject::ByteArray(b) => b.clone(),
            _ => return Err(PyError::type_error("b64encode() argument must be bytes")),
        };
        Ok(PyObjectRef::imm(PyObject::Bytes(b64_encode(&bytes).into_bytes())))
    });

    // encodebytes/decodebytes: the legacy MIME-oriented form base64.b64encode
    // is built on top of in real CPython — same alphabet, but wraps output
    // every 76 characters (57 input bytes) with a trailing newline, and
    // operates on bytes in/out rather than str. Needed directly by
    // `email/encoders.py` (`from base64 import encodebytes`), a completely
    // ordinary use of the real stdlib `base64` module, not anything
    // email-specific about the encoding itself.
    b64_func!("encodebytes", |args| {
        if args.len() != 1 { return Err(PyError::type_error("encodebytes() takes exactly one argument")); }
        let data = args[0].borrow();
        let bytes = match &*data {
            PyObject::Bytes(b) => b.clone(),
            PyObject::ByteArray(b) => b.clone(),
            _ => return Err(PyError::type_error("encodebytes() argument must be bytes")),
        };
        let mut out = String::new();
        for chunk in bytes.chunks(57) {
            out.push_str(&b64_encode(chunk));
            out.push('\n');
        }
        Ok(PyObjectRef::imm(PyObject::Bytes(out.into_bytes())))
    });

    b64_func!("decodebytes", |args| {
        if args.len() != 1 { return Err(PyError::type_error("decodebytes() takes exactly one argument")); }
        let data = args[0].borrow();
        let bytes = match &*data {
            PyObject::Bytes(b) => b.clone(),
            PyObject::ByteArray(b) => b.clone(),
            _ => return Err(PyError::type_error("decodebytes() argument must be bytes")),
        };
        let s: String = bytes.iter().filter(|b| !b.is_ascii_whitespace()).map(|b| *b as char).collect();
        match b64_decode(&s) {
            Ok(bytes) => Ok(PyObjectRef::imm(PyObject::Bytes(bytes))),
            Err(e) => Err(PyError::value_error(e)),
        }
    });

    // Accepts `bytes` too (not just `str`) — real `base64.b64decode` does,
    // and now that `b64encode` correctly returns `bytes` (fixed above),
    // `b64decode(b64encode(x))` must round-trip without the caller having
    // to manually `.decode('ascii')` in between.
    b64_func!("b64decode", |args| {
        if args.len() != 1 { return Err(PyError::type_error("b64decode() takes exactly one argument")); }
        let data = args[0].borrow();
        let s = match &*data {
            PyObject::Str(s) => s.to_string(),
            PyObject::Bytes(b) => String::from_utf8_lossy(b).to_string(),
            _ => return Err(PyError::type_error("b64decode() argument must be a string or bytes")),
        };
        match b64_decode(&s) {
            Ok(bytes) => Ok(PyObjectRef::imm(PyObject::Bytes(bytes))),
            Err(e) => Err(PyError::value_error(e)),
        }
    });

    // `base64.b32encode`/`b32decode` (RFC 4648 Base32) were missing
    // entirely — same general shape as the existing hand-written base64
    // codec just above, reused rather than pulling in a new dependency.
    const B32_ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

    fn b32_encode(data: &[u8]) -> String {
        let mut out = String::new();
        for chunk in data.chunks(5) {
            let mut buf = [0u8; 5];
            buf[..chunk.len()].copy_from_slice(chunk);
            let n = ((buf[0] as u64) << 32) | ((buf[1] as u64) << 24) | ((buf[2] as u64) << 16) | ((buf[3] as u64) << 8) | (buf[4] as u64);
            // Number of valid output characters (before padding) for a
            // partial final chunk, per RFC 4648's own table.
            let out_chars = match chunk.len() { 1 => 2, 2 => 4, 3 => 5, 4 => 7, _ => 8 };
            for i in 0..8 {
                if i < out_chars {
                    let shift = 35 - i * 5;
                    let idx = ((n >> shift) & 0x1F) as usize;
                    out.push(B32_ALPHABET[idx] as char);
                } else {
                    out.push('=');
                }
            }
        }
        out
    }

    fn b32_decode(s: &str) -> Result<Vec<u8>, String> {
        let mut rev = [255u8; 256];
        for (i, &c) in B32_ALPHABET.iter().enumerate() {
            rev[c as usize] = i as u8;
        }
        let s = s.trim_end_matches('=');
        let mut out = Vec::new();
        for chunk in s.as_bytes().chunks(8) {
            let mut n: u64 = 0;
            let mut valid_bits = 0;
            for &c in chunk {
                let v = rev[c as usize];
                if v == 255 { return Err("Invalid base32 character".to_string()); }
                n = (n << 5) | v as u64;
                valid_bits += 5;
            }
            // Left-align the accumulated bits within a 40-bit window, then
            // emit one byte per complete group of 8 bits.
            n <<= 40 - valid_bits;
            let total_bytes = valid_bits / 8;
            for i in 0..total_bytes {
                out.push(((n >> (32 - i * 8)) & 0xFF) as u8);
            }
        }
        Ok(out)
    }

    b64_func!("b32encode", |args| {
        if args.len() != 1 { return Err(PyError::type_error("b32encode() takes exactly one argument")); }
        let data = match &*args[0].borrow() {
            PyObject::Bytes(b) => b.clone(),
            PyObject::ByteArray(b) => b.clone(),
            _ => return Err(PyError::type_error("b32encode() argument must be bytes")),
        };
        Ok(PyObjectRef::imm(PyObject::Bytes(b32_encode(&data).into_bytes())))
    });

    b64_func!("b32decode", |args| {
        if args.len() != 1 { return Err(PyError::type_error("b32decode() takes exactly one argument")); }
        let s = match &*args[0].borrow() {
            PyObject::Str(s) => s.to_string(),
            PyObject::Bytes(b) => String::from_utf8_lossy(b).to_string(),
            _ => return Err(PyError::type_error("b32decode() argument must be a string or bytes")),
        };
        match b32_decode(&s) {
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

        type_dict.insert_str("digest", PyObjectRef::new(PyObject::BuiltinFunction {
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

        type_dict.insert_str("hexdigest", PyObjectRef::new(PyObject::BuiltinFunction {
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

pub fn create_zlib_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! z_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    // `zlib.compress`/`decompress` were complete no-op STUBS — returned the
    // input bytes completely UNCHANGED, silently claiming to "compress"
    // without doing anything at all. This wasn't just a missing-feature
    // gap: any code round-tripping through `zlib.compress`/`decompress`
    // itself never noticed (garbage in, same garbage out), but real
    // interop with ACTUAL zlib-compressed data from anywhere else (a file,
    // a network payload, `pickle`'s own optional compression, `gzip`
    // internals) would either silently produce bogus "decompressed"
    // output or fail outright. `flate2` (this project's own existing
    // dependency, already used for the real `gzip` module — see
    // `modules/files.rs`) provides a dedicated zlib encoder/decoder, not
    // just the gzip-framed one — wiring it in here was a small, contained
    // fix reusing infrastructure that already existed for a different
    // module.
    z_func!("compress", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("compress() missing required argument (data)"));
        }
        let data = match &*args[0].borrow() {
            PyObject::Bytes(b) => b.clone(),
            PyObject::ByteArray(b) => b.clone(),
            _ => return Err(PyError::type_error("compress() argument must be bytes")),
        };
        let level = if args.len() > 1 { args[1].as_i64().unwrap_or(6).clamp(0, 9) as u32 } else { 6 };
        use std::io::Write;
        let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::new(level));
        encoder.write_all(&data).map_err(|e| PyError::OsError(format!("{}", e)))?;
        let compressed = encoder.finish().map_err(|e| PyError::OsError(format!("{}", e)))?;
        Ok(PyObjectRef::imm(PyObject::Bytes(compressed)))
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
        use std::io::Read;
        let mut decoder = flate2::read::ZlibDecoder::new(&data[..]);
        let mut out = Vec::new();
        decoder.read_to_end(&mut out).map_err(|e| PyError::value_error(format!("Error -3 while decompressing data: {}", e)))?;
        Ok(PyObjectRef::imm(PyObject::Bytes(out)))
    });

    d
}

