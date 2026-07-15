use crate::object::*;
use std::collections::HashMap;

/// CRC-32 lookup table
fn make_crc32_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    for i in 0..256u32 {
        let mut crc = i;
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = 0xedb88320 ^ (crc >> 1);
            } else {
                crc >>= 1;
            }
        }
        table[i as usize] = crc;
    }
    table
}

fn crc32_impl(data: &[u8]) -> u32 {
    let table = make_crc32_table();
    let mut crc = 0xffffffffu32;
    for &byte in data {
        let idx = ((crc ^ byte as u32) & 0xff) as usize;
        crc = table[idx] ^ (crc >> 8);
    }
    crc ^ 0xffffffff
}

/// Standard base64 alphabet (RFC 4648)
const BASE64_CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn b64_encode(data: &[u8]) -> String {
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let len = chunk.len();
        let b0 = chunk[0];
        let b1 = if len > 1 { chunk[1] } else { 0 };
        let b2 = if len > 2 { chunk[2] } else { 0 };
        out.push(BASE64_CHARS[(b0 >> 2) as usize] as char);
        out.push(BASE64_CHARS[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
        if len > 1 {
            out.push(BASE64_CHARS[(((b1 & 0x0F) << 2) | (b2 >> 6)) as usize] as char);
        } else {
            out.push('=');
        }
        if len > 2 {
            out.push(BASE64_CHARS[(b2 & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

fn b64_decode(s: &[u8]) -> Result<Vec<u8>, String> {
    // Build reverse lookup
    let mut rev = [255u8; 256];
    for (i, &c) in BASE64_CHARS.iter().enumerate() {
        rev[c as usize] = i as u8;
    }
    // Strip whitespace
    let clean: Vec<u8> = s.iter()
        .filter(|&&b| !b.is_ascii_whitespace())
        .copied()
        .collect();
    if clean.is_empty() {
        return Ok(vec![]);
    }
    if clean.len() % 4 != 0 {
        return Err("Invalid base64 input length".to_string());
    }
    let mut out = Vec::new();
    for chunk in clean.chunks(4) {
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

/// UUencode/decode helpers
fn uu_encode(data: &[u8]) -> String {
    let mut out = String::new();
    for chunk in data.chunks(45) {
        let len = chunk.len();
        out.push((len as u8 + 32) as char);
        for triple in chunk.chunks(3) {
            let tlen = triple.len();
            let b0 = triple[0];
            let b1 = if tlen > 1 { triple[1] } else { 0 };
            let b2 = if tlen > 2 { triple[2] } else { 0 };
            out.push(((b0 >> 2) + 32) as u8 as char);
            out.push((((b0 & 0x03) << 4 | b1 >> 4) + 32) as u8 as char);
            if tlen > 1 {
                out.push((((b1 & 0x0f) << 2 | b2 >> 6) + 32) as u8 as char);
            } else {
                out.push(96u8 as char); // backtick for padding
            }
            if tlen > 2 {
                out.push(((b2 & 0x3f) + 32) as u8 as char);
            } else {
                out.push(96u8 as char);
            }
        }
        out.push('\n');
    }
    if !out.is_empty() {
        out.push_str("`\n");
    }
    out
}

fn uu_decode(data: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let lines: Vec<&[u8]> = data
        .split(|&b| b == b'\n' || b == b'\r')
        .filter(|l| !l.is_empty())
        .collect();
    for line in &lines {
        if line.is_empty() || line[0] == b'`' {
            continue;
        }
        let line_len = (line[0] as usize).wrapping_sub(32);
        if line_len > 45 {
            continue;
        }
        let mut buf = Vec::new();
        let mut i = 1;
        while i < line.len() && buf.len() < line_len {
            if i + 1 >= line.len() {
                break;
            }
            let c1 = line[i].wrapping_sub(32) & 0x3f;
            let c2 = line[i + 1].wrapping_sub(32) & 0x3f;
            buf.push((c1 << 2) | (c2 >> 4));
            if i + 2 < line.len() && buf.len() < line_len {
                let c3 = line[i + 2].wrapping_sub(32) & 0x3f;
                buf.push((c2 << 4) | (c3 >> 2));
                if i + 3 < line.len() && buf.len() < line_len {
                    let c4 = line[i + 3].wrapping_sub(32) & 0x3f;
                    buf.push((c3 << 6) | c4);
                }
            }
            i += 4;
        }
        out.extend_from_slice(&buf[..line_len.min(buf.len())]);
    }
    Ok(out)
}

pub fn create_binascii_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();

    macro_rules! bin_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
                name: $name.to_string(), func: $func,
            }));
        };
    }

    // --- hexlify ---
    bin_func!("hexlify", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("hexlify() missing required argument"));
        }
        let data = match &*args[0].borrow() {
            PyObject::Bytes(b) => b.clone(),
            PyObject::ByteArray(b) => b.clone(),
            PyObject::Str(s) => s.as_bytes().to_vec(),
            _ => return Err(PyError::type_error("argument must be bytes, bytearray, or str")),
        };
        let hex: String = data.iter().map(|b| format!("{:02x}", b)).collect();
        Ok(PyObjectRef::imm(PyObject::Bytes(hex.into_bytes())))
    });

    // --- unhexlify ---
    bin_func!("unhexlify", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("unhexlify() missing required argument"));
        }
        let hex_str = match &*args[0].borrow() {
            PyObject::Bytes(b) => String::from_utf8_lossy(b).to_string(),
            PyObject::ByteArray(b) => String::from_utf8_lossy(b).to_string(),
            PyObject::Str(s) => s.clone(),
            _ => return Err(PyError::type_error("argument must be bytes, bytearray, or str")),
        };
        let clean: String = hex_str.chars().filter(|c| !c.is_whitespace()).collect();
        if clean.len() % 2 != 0 {
            return Err(PyError::value_error("hex string must be of even length"));
        }
        let bytes: Vec<u8> = (0..clean.len()).step_by(2)
            .map(|i| u8::from_str_radix(&clean[i..i+2], 16))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| PyError::value_error("non-hexadecimal number found"))?;
        Ok(PyObjectRef::imm(PyObject::Bytes(bytes)))
    });

    // --- a2b_base64 ---
    bin_func!("a2b_base64", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("a2b_base64() missing required argument"));
        }
        let data = match &*args[0].borrow() {
            PyObject::Bytes(b) => b.clone(),
            PyObject::ByteArray(b) => b.clone(),
            PyObject::Str(s) => s.as_bytes().to_vec(),
            _ => return Err(PyError::type_error("argument must be bytes, bytearray, or str")),
        };
        match b64_decode(&data) {
            Ok(bytes) => Ok(PyObjectRef::imm(PyObject::Bytes(bytes))),
            Err(e) => Err(PyError::value_error(e)),
        }
    });

    // --- b2a_base64 ---
    bin_func!("b2a_base64", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("b2a_base64() missing required argument"));
        }
        let data = match &*args[0].borrow() {
            PyObject::Bytes(b) => b.clone(),
            PyObject::ByteArray(b) => b.clone(),
            _ => return Err(PyError::type_error("argument must be bytes or bytearray")),
        };
        let mut encoded = b64_encode(&data);
        encoded.push('\n');
        Ok(PyObjectRef::imm(PyObject::Bytes(encoded.into_bytes())))
    });

    // --- a2b_uu ---
    bin_func!("a2b_uu", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("a2b_uu() missing required argument"));
        }
        let data = match &*args[0].borrow() {
            PyObject::Bytes(b) => b.clone(),
            PyObject::Str(s) => s.as_bytes().to_vec(),
            _ => return Err(PyError::type_error("argument must be bytes or str")),
        };
        match uu_decode(&data) {
            Ok(bytes) => Ok(PyObjectRef::imm(PyObject::Bytes(bytes))),
            Err(e) => Err(PyError::value_error(e)),
        }
    });

    // --- b2a_uu ---
    bin_func!("b2a_uu", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("b2a_uu() missing required argument"));
        }
        let data = match &*args[0].borrow() {
            PyObject::Bytes(b) => b.clone(),
            PyObject::ByteArray(b) => b.clone(),
            _ => return Err(PyError::type_error("argument must be bytes or bytearray")),
        };
        let encoded = uu_encode(&data);
        Ok(PyObjectRef::imm(PyObject::Bytes(encoded.into_bytes())))
    });

    // --- crc32 ---
    bin_func!("crc32", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("crc32() missing required argument"));
        }
        let data = match &*args[0].borrow() {
            PyObject::Bytes(b) => b.clone(),
            PyObject::ByteArray(b) => b.clone(),
            _ => return Err(PyError::type_error("argument must be bytes or bytearray")),
        };
        // Optional second argument: initial CRC (not implemented, Python's binascii.crc32 doesn't use it in simple cases)
        Ok(py_int(crc32_impl(&data) as i64))
    });

    // --- a2b_qp (stub) ---
    bin_func!("a2b_qp", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("a2b_qp() missing required argument"));
        }
        let data = match &*args[0].borrow() {
            PyObject::Bytes(b) => b.clone(),
            PyObject::Str(s) => s.as_bytes().to_vec(),
            _ => return Err(PyError::type_error("argument must be bytes or str")),
        };
        // Simple quoted-printable decoder: handle =XX hex escapes
        let mut out = Vec::new();
        let mut i = 0;
        while i < data.len() {
            if data[i] == b'=' && i + 2 < data.len() && data[i+1] != b'\n' {
                if data[i+1] == b'\r' || data[i+1] == b'\n' {
                    // Soft line break, skip
                    i += 2;
                    if data[i] == b'\n' { i += 1; }
                    continue;
                }
                if let Ok(byte) = u8::from_str_radix(
                    &String::from_utf8_lossy(&data[i+1..i+3]),
                    16
                ) {
                    out.push(byte);
                    i += 3;
                    continue;
                }
            }
            out.push(data[i]);
            i += 1;
        }
        Ok(PyObjectRef::imm(PyObject::Bytes(out)))
    });

    // --- b2a_qp (stub) ---
    bin_func!("b2a_qp", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("b2a_qp() missing required argument"));
        }
        let data = match &*args[0].borrow() {
            PyObject::Bytes(b) => b.clone(),
            PyObject::ByteArray(b) => b.clone(),
            _ => return Err(PyError::type_error("argument must be bytes or bytearray")),
        };
        // Simple quoted-printable encoder: encode non-printable and non-ASCII bytes
        let mut out = Vec::new();
        for &b in &data {
            if b == b'=' {
                out.extend_from_slice(b"=3D");
            } else if b == b'\t' || b == b' ' {
                out.push(b);
            } else if b.is_ascii_graphic() || b == b' ' {
                out.push(b);
            } else if b == b'\n' {
                out.push(b'\n');
            } else if b == b'\r' {
                out.push(b'\r');
            } else {
                out.push(b'=');
                out.push(b"0123456789ABCDEF"[((b >> 4) & 0x0f) as usize]);
                out.push(b"0123456789ABCDEF"[(b & 0x0f) as usize]);
            }
        }
        Ok(PyObjectRef::imm(PyObject::Bytes(out)))
    });

    // --- rlecode_hqx (stub) ---
    bin_func!("rlecode_hqx", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("rlecode_hqx() missing required argument"));
        }
        let data = match &*args[0].borrow() {
            PyObject::Bytes(b) => b.clone(),
            PyObject::ByteArray(b) => b.clone(),
            _ => return Err(PyError::type_error("argument must be bytes or bytearray")),
        };
        // Simplified: return data as-is (no RLE encoding)
        Ok(PyObjectRef::imm(PyObject::Bytes(data)))
    });

    // --- rledecode_hqx (stub) ---
    bin_func!("rledecode_hqx", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("rledecode_hqx() missing required argument"));
        }
        let data = match &*args[0].borrow() {
            PyObject::Bytes(b) => b.clone(),
            PyObject::ByteArray(b) => b.clone(),
            _ => return Err(PyError::type_error("argument must be bytes or bytearray")),
        };
        // Simplified: return data as-is (no RLE decoding)
        Ok(PyObjectRef::imm(PyObject::Bytes(data)))
    });

    d
}
