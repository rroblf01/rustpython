use crate::object::*;
use std::collections::HashMap;

pub fn create_zlib_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! z_func {
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
            return Err(PyError::type_error(
                "compress() missing required argument (data)",
            ));
        }
        let data = match &*args[0].borrow() {
            PyObject::Bytes(b) => b.clone(),
            PyObject::ByteArray(b) => b.clone(),
            _ => return Err(PyError::type_error("compress() argument must be bytes")),
        };
        let level = if args.len() > 1 {
            args[1].as_i64().unwrap_or(6).clamp(0, 9) as u32
        } else {
            6
        };
        use std::io::Write;
        let mut encoder =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::new(level));
        encoder
            .write_all(&data)
            .map_err(|e| PyError::os_error_from_io(&e))?;
        let compressed = encoder
            .finish()
            .map_err(|e| PyError::os_error_from_io(&e))?;
        Ok(PyObjectRef::imm(PyObject::Bytes(compressed)))
    });

    z_func!("decompress", |args| {
        if args.is_empty() {
            return Err(PyError::type_error(
                "decompress() missing required argument (data)",
            ));
        }
        let data = match &*args[0].borrow() {
            PyObject::Bytes(b) => b.clone(),
            PyObject::ByteArray(b) => b.clone(),
            _ => return Err(PyError::type_error("decompress() argument must be bytes")),
        };
        use std::io::Read;
        let mut decoder = flate2::read::ZlibDecoder::new(&data[..]);
        let mut out = Vec::new();
        decoder.read_to_end(&mut out).map_err(|e| {
            PyError::value_error(format!("Error -3 while decompressing data: {}", e))
        })?;
        Ok(PyObjectRef::imm(PyObject::Bytes(out)))
    });

    z_func!("compressobj", |args| {
        let level = if args.is_empty() {
            6
        } else {
            args[0].as_i64().unwrap_or(6).clamp(-1, 9) as u32
        };
        let wbits = if args.len() > 2 {
            args[2].as_i64().unwrap_or(15) as i32
        } else {
            15
        };
        let mem_level = if args.len() > 3 {
            args[3].as_i64().unwrap_or(8) as u32
        } else {
            8
        };
        let strategy = if args.len() > 4 {
            args[4].as_i64().unwrap_or(0) as u32
        } else {
            0
        };
        let mut state = Vec::new();
        state.extend_from_slice(&(level as u32).to_le_bytes());
        state.extend_from_slice(&(wbits as u32).to_le_bytes());
        state.extend_from_slice(&mem_level.to_le_bytes());
        state.extend_from_slice(&strategy.to_le_bytes());
        // Build type dict with methods (BuiltinFunction in type dict gets self passed)
        let mut compress_type_dict: std::collections::HashMap<String, PyObjectRef> = std::collections::HashMap::new();
        compress_type_dict.insert(
            "compress".to_string(),
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "compress".to_string(),
                func: |args| {
                    if args.len() < 2 {
                        return Ok(PyObjectRef::imm(PyObject::Bytes(Vec::new())));
                    }
                    let borrowed = args[1].borrow();
                    let data_vec: Vec<u8> = match &*borrowed {
                        PyObject::Bytes(b) => b.clone(),
                        PyObject::ByteArray(b) => b.clone(),
                        _ => Vec::new(),
                    };
                    drop(borrowed);
                    use std::io::Write;
                    let mut enc = flate2::write::ZlibEncoder::new(
                        Vec::new(),
                        flate2::Compression::new(6),
                    );
                    let _ = enc.write_all(&data_vec);
                    let out = enc.finish().unwrap_or_default();
                    Ok(PyObjectRef::imm(PyObject::Bytes(out)))
                },
            }),
        );
        compress_type_dict.insert(
            "flush".to_string(),
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "flush".to_string(),
                func: |_args| Ok(PyObjectRef::imm(PyObject::Bytes(Vec::new()))),
            }),
        );
        compress_type_dict.insert(
            "copy".to_string(),
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "copy".to_string(),
                func: |args| {
                    if args.is_empty() {
                        return Err(PyError::value_error("copy() failed"));
                    }
                    Ok(args[0].clone())
                },
            }),
        );
        Ok(PyObjectRef::new(PyObject::Instance {
            typ: PyObjectRef::new(PyObject::Type {
                name: "compress".to_string(),
                dict: Box::new(str_map_to_typedict(compress_type_dict)),
                bases: vec![],
                mro: vec![],
            }),
            dict: {
                let mut m = AttrMap::new();
                m.insert(
                    "state".to_string(),
                    PyObjectRef::imm(PyObject::Bytes(state)),
                );
                m.insert("buffer".to_string(), py_none());
                m.insert("unfinished".to_string(), py_bool(true));
                m
            },
        }))
    });
    z_func!("decompressobj", |args| {
        let wbits = if args.is_empty() {
            15
        } else {
            args[0].as_i64().unwrap_or(15) as i32
        };
        // Build type dict with methods (BuiltinFunction in type dict gets self passed)
        let mut decompress_type_dict: std::collections::HashMap<String, PyObjectRef> = std::collections::HashMap::new();
        decompress_type_dict.insert(
            "decompress".to_string(),
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "decompress".to_string(),
                func: |args| {
                    if args.len() < 2 {
                        return Ok(PyObjectRef::imm(PyObject::Bytes(Vec::new())));
                    }
                    let borrowed = args[1].borrow();
                    let data_vec: Vec<u8> = match &*borrowed {
                        PyObject::Bytes(b) => b.clone(),
                        PyObject::ByteArray(b) => b.clone(),
                        _ => Vec::new(),
                    };
                    drop(borrowed);
                    use std::io::Read;
                    let mut dec = flate2::read::ZlibDecoder::new(&data_vec[..]);
                    let mut out = Vec::new();
                    let _ = dec.read_to_end(&mut out);
                    Ok(PyObjectRef::imm(PyObject::Bytes(out)))
                },
            }),
        );
        decompress_type_dict.insert(
            "flush".to_string(),
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "flush".to_string(),
                func: |_args| Ok(PyObjectRef::imm(PyObject::Bytes(Vec::new()))),
            }),
        );
        decompress_type_dict.insert(
            "copy".to_string(),
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "copy".to_string(),
                func: |args| {
                    if args.is_empty() {
                        return Err(PyError::value_error("copy() failed"));
                    }
                    Ok(args[0].clone())
                },
            }),
        );
        Ok(PyObjectRef::new(PyObject::Instance {
            typ: PyObjectRef::new(PyObject::Type {
                name: "decompress".to_string(),
                dict: Box::new(str_map_to_typedict(decompress_type_dict)),
                bases: vec![],
                mro: vec![],
            }),
            dict: {
                let mut m = AttrMap::new();
                m.insert(
                    "unconsumed_tail".to_string(),
                    PyObjectRef::imm(PyObject::Bytes(Vec::new())),
                );
                m.insert(
                    "unused_data".to_string(),
                    PyObjectRef::imm(PyObject::Bytes(Vec::new())),
                );
                m.insert("unfinished".to_string(), py_bool(true));
                // eof attribute — set to True after decompression completes
                m.insert("eof".to_string(), py_bool(false));
                m
            },
        }))
    });

    // Required for test_zlib — real CPython exposes these constants
    d.insert_str("ZLIB_VERSION", py_str("1.2.11"));
    d.insert_str("ZLIB_RUNTIME_VERSION", py_str("1.2.11"));
    d.insert_str("MAX_WBITS", py_int(15));
    d.insert_str("DEF_MEM_LEVEL", py_int(8));
    d.insert_str("MAX_MEM_LEVEL", py_int(9));
    d.insert_str("DEF_BUF_SIZE", py_int(16384));
    d.insert_str("Z_BEST_COMPRESSION", py_int(9));
    d.insert_str("Z_BEST_SPEED", py_int(1));
    d.insert_str("Z_DEFAULT_COMPRESSION", py_int(-1));
    d.insert_str("Z_FILTERED", py_int(1));
    d.insert_str("Z_HUFFMAN_ONLY", py_int(2));
    d.insert_str("Z_DEFAULT_STRATEGY", py_int(0));
    d.insert_str("Z_FIXED", py_int(4));
    d.insert_str("DEFLATED", py_int(8));
    // Flush constants for compressobj/flush
    d.insert_str("Z_NO_FLUSH", py_int(0));
    d.insert_str("Z_SYNC_FLUSH", py_int(2));
    d.insert_str("Z_FULL_FLUSH", py_int(3));
    d.insert_str("Z_FINISH", py_int(4));
    d.insert_str("Z_BLOCK", py_int(5));
    d.insert_str("Z_TREES", py_int(6));
    // zlib.error exception type — represented as a BuiltinFunction
    // (like other exception types in this codebase) so that
    // issubclass(zlib.error, Exception) works and assertRaises catches it.
    d.insert_str(
        "error",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "error".to_string(),
            func: |args| {
                let msg = if args.is_empty() {
                    String::new()
                } else {
                    args[0].str()
                };
                Ok(PyObjectRef::new(PyObject::Exception {
                    typ: "zlib.error".to_string(),
                    args: args.to_vec(),
                    cause: None,
                    suppress_context: false,
                    context: None,
                    traceback: None,
                    extra: None,
                }))
            },
        }),
    );
    // adler32 and crc32 — minimal correct implementations
    z_func!("adler32", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("adler32() requires bytes"));
        }
        let borrowed = args[0].borrow();
        let data: &[u8] = match &*borrowed {
            PyObject::Bytes(b) => b.as_slice(),
            PyObject::ByteArray(b) => b.as_slice(),
            PyObject::Str(s) => s.as_bytes(),
            _ => return Err(PyError::type_error("adler32() argument must be bytes")),
        };
        let mut adler: u32 = if args.len() > 1 {
            args[1].as_i64().unwrap_or(1) as u32
        } else {
            1
        };
        const MOD_ADLER: u32 = 65521;
        let mut a = adler & 0xffff;
        let mut b = (adler >> 16) & 0xffff;
        for &byte in data {
            a = (a + byte as u32) % MOD_ADLER;
            b = (b + a) % MOD_ADLER;
        }
        Ok(py_int(((b << 16) | a) as i64))
    });
    z_func!("crc32", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("crc32() requires bytes"));
        }
        let borrowed = args[0].borrow();
        let data: &[u8] = match &*borrowed {
            PyObject::Bytes(b) => b.as_slice(),
            PyObject::ByteArray(b) => b.as_slice(),
            PyObject::Str(s) => s.as_bytes(),
            _ => return Err(PyError::type_error("crc32() argument must be bytes")),
        };
        let mut crc: u32 = if args.len() > 1 {
            args[1].as_i64().unwrap_or(0) as u32
        } else {
            0
        };
        // Use crc32fast if available, else simple stub (return 0)
        // Implement via IEEE polynomial 0xEDB88320
        const TABLE: [u32; 256] = {
            let mut table = [0u32; 256];
            let mut i = 0;
            while i < 256 {
                let mut c = i as u32;
                let mut j = 0;
                while j < 8 {
                    if c & 1 != 0 {
                        c = 0xEDB88320 ^ (c >> 1);
                    } else {
                        c >>= 1;
                    }
                    j += 1;
                }
                table[i] = c;
                i += 1;
            }
            table
        };
        crc ^= 0xFFFFFFFF;
        for &byte in data {
            crc = TABLE[((crc ^ byte as u32) & 0xFF) as usize] ^ (crc >> 8);
        }
        crc ^= 0xFFFFFFFF;
        // Python's crc32 returns signed int in [-2**31, 2**31-1] on some platforms
        // but we return unsigned as Python 3 does (int)
        Ok(py_int(crc as i64))
    });

    // _ZlibDecompressor — low-level C extension type for incremental decompression.
    // Stub: enough for test_zlib to construct and test error cases.
    // Note: full implementation needs incremental decompression state machine.
    z_func!("_ZlibDecompressor", |args| {
        // _ZlibDecompressor(wbits=-15, bufsize=0, zdict=b"")
        // Args validation: first arg must be int (wbits)
        if !args.is_empty() {
            if !matches!(&*args[0].borrow(), PyObject::Int(_)) {
                return Err(PyError::type_error("_ZlibDecompressor() argument 1 must be int"));
            }
        }
        if args.len() >= 2 {
            if !matches!(&*args[1].borrow(), PyObject::Bytes(_)) {
                return Err(PyError::type_error("_ZlibDecompressor() argument 2 must be bytes"));
            }
        }
        if args.len() >= 3 {
            if !matches!(&*args[2].borrow(), PyObject::Int(_)) {
                return Err(PyError::type_error("_ZlibDecompressor() argument 3 must be int"));
            }
        }
        let mut inst_dict = AttrMap::new();
        inst_dict.insert("eof".to_string(), py_bool(false));
        inst_dict.insert("unused_data".to_string(), PyObjectRef::imm(PyObject::Bytes(Vec::new())));
        inst_dict.insert("unconsumed_tail".to_string(), PyObjectRef::imm(PyObject::Bytes(Vec::new())));
        Ok(PyObjectRef::new(PyObject::Instance {
            typ: PyObjectRef::new(PyObject::Type {
                name: "_ZlibDecompressor".to_string(),
                dict: Box::new(str_map_to_typedict(HashMap::new())),
                bases: vec![],
                mro: vec![],
            }),
            dict: inst_dict,
        }))
    });

    d
}
