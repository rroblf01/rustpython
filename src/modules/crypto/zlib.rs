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
        Ok(PyObjectRef::new(PyObject::Instance {
            typ: PyObjectRef::new(PyObject::Type {
                name: "compress".to_string(),
                dict: Box::new(str_map_to_typedict(HashMap::new())),
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
        Ok(PyObjectRef::new(PyObject::Instance {
            typ: PyObjectRef::new(PyObject::Type {
                name: "decompress".to_string(),
                dict: Box::new(str_map_to_typedict(HashMap::new())),
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
                m
            },
        }))
    });

    d
}
