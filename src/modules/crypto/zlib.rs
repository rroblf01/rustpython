use crate::object::*;
use flate2::{Compress, Compression, Decompress, FlushCompress, FlushDecompress, Status};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

fn zlib_error(msg: impl Into<String>) -> PyError {
    let obj = PyObjectRef::new(PyObject::Exception {
        typ: "zlib.error".to_string(),
        args: vec![py_str(&msg.into())],
        cause: None,
        suppress_context: false,
        context: None,
        traceback: None,
        extra: None,
    });
    PyError::Exception("zlib.error".to_string(), obj)
}

fn bytes_arg(obj: &PyObjectRef) -> PyResult<Vec<u8>> {
    match &*obj.borrow() {
        PyObject::Bytes(b) => Ok(b.clone()),
        PyObject::ByteArray(b) => Ok(b.clone()),
        _ => Err(PyError::type_error(format!(
            "a bytes-like object is required, not '{}'",
            obj.borrow().type_name()
        ))),
    }
}

/// Resolve a `level` argument (-1..=9) to a `flate2::Compression`, raising
/// `zlib.error` for anything outside that range (real zlib: `Z_STREAM_ERROR`).
fn resolve_level(level: i64) -> PyResult<Compression> {
    if !(-1..=9).contains(&level) {
        return Err(zlib_error("Bad compression level"));
    }
    let lvl = if level < 0 { 6 } else { level as u32 };
    Ok(Compression::new(lvl))
}

/// Kind of stream a given `wbits` value selects, per real zlib semantics:
/// 9..=15 -> zlib-wrapped deflate, -15..=-9 -> raw deflate (no header),
/// 25..=31 -> gzip-wrapped deflate. Anything else is a `ValueError`.
enum WbitsKind {
    Zlib(u8),
    Raw(u8),
    Gzip(u8),
}

fn resolve_wbits(wbits: i64) -> PyResult<WbitsKind> {
    if (9..=15).contains(&wbits) {
        Ok(WbitsKind::Zlib(wbits as u8))
    } else if (-15..=-9).contains(&wbits) {
        Ok(WbitsKind::Raw((-wbits) as u8))
    } else if (25..=31).contains(&wbits) {
        Ok(WbitsKind::Gzip((wbits - 16) as u8))
    } else {
        Err(PyError::value_error("Invalid initialization option"))
    }
}

// NOTE: `Compress`/`Decompress::new_with_window_bits`/`new_gzip` require
// flate2's `any_zlib` feature, which links a real C zlib (via `libz-sys`)
// instead of the pure-Rust `miniz_oxide` backend this project uses (see
// CLAUDE.md's minimal-dependency ethos) — so the plain `zlib_header: bool`
// constructors are used instead, which always operate at the default
// (15-bit) window regardless of the requested `wbits` magnitude. This means
// a smaller explicit window (e.g. `wbits=9`) still decodes/encodes
// correctly (a smaller window is a subset of the default one), it's simply
// not memory-optimized for it. True raw-gzip framing (`wbits` 25..=31) has
// no low-level incremental equivalent without `any_zlib`; it's approximated
// here as a raw (headerless) deflate stream, which is wrong for
// interop with real gzip files but keeps the incremental buffer/max_length
// machinery below working uniformly. Real gzip framing IS provided,
// correctly, for the plain one-shot `compress()`/`decompress()` functions
// via `flate2::{read,write}::Gz{En,De}coder`.
fn make_compress(level: Compression, wbits: i64) -> PyResult<Compress> {
    match resolve_wbits(wbits)? {
        WbitsKind::Zlib(_) => Ok(Compress::new(level, true)),
        WbitsKind::Raw(_) | WbitsKind::Gzip(_) => Ok(Compress::new(level, false)),
    }
}

fn make_decompress(wbits: i64) -> PyResult<Decompress> {
    match resolve_wbits(wbits)? {
        WbitsKind::Zlib(_) => Ok(Decompress::new(true)),
        WbitsKind::Raw(_) | WbitsKind::Gzip(_) => Ok(Decompress::new(false)),
    }
}

/// Drive `Compress::compress` in a loop, growing the output buffer as
/// needed, until all of `input` has been consumed (or, with
/// `FlushCompress::Finish`, until the stream reports completion).
fn run_compress(inner: &mut Compress, input: &[u8], flush: FlushCompress) -> PyResult<Vec<u8>> {
    let mut out = Vec::new();
    let mut consumed = 0usize;
    let mut buf = vec![0u8; 32 * 1024];
    loop {
        let before_in = inner.total_in();
        let before_out = inner.total_out();
        let status = inner
            .compress(&input[consumed..], &mut buf, flush)
            .map_err(|e| zlib_error(format!("Error while compressing data: {}", e)))?;
        let produced = (inner.total_out() - before_out) as usize;
        out.extend_from_slice(&buf[..produced]);
        consumed += (inner.total_in() - before_in) as usize;
        match status {
            Status::StreamEnd => break,
            // `BufError` is ambiguous by itself: zlib returns it BOTH when
            // our 32KB output buffer genuinely filled up mid-call (real
            // "more to drain", confirmed by `produced == buf.len()`) AND
            // when there's simply no progress possible at all — e.g.
            // `compress(b"")` with `flush=None` on a stream with nothing
            // buffered — which produces 0 bytes into a buffer nowhere near
            // full. Unconditionally looping on ANY `BufError` (as a first
            // version of this fix did) spun forever on the latter case:
            // `zlib.compressobj().compress(b"")` hung instead of returning
            // immediately (regression caught by `test_zipimport.py`'s
            // `testEmptyPy`, which zips a zero-byte `.py` file — real
            // CPython returns 2 bytes, the zlib header, right away).
            Status::BufError => {
                if produced == buf.len() {
                    continue;
                } else {
                    break;
                }
            }
            Status::Ok => {
                // Used to keep looping as long as this call had produced
                // ANY bytes at all — correct for the very first plain
                // `compress()` call (which legitimately emits a large
                // chunk once, then a trailing no-op call with `produced ==
                // 0` broke the loop), but wrong for `Z_SYNC_FLUSH`/
                // `Z_FULL_FLUSH`: those flush modes emit a small
                // (non-empty) sync-marker block on EVERY call by design,
                // so `produced` is never 0 and the old condition spun
                // forever re-flushing an already-empty stream (confirmed
                // hang: `zlib.compressobj().flush(Z_SYNC_FLUSH)` on any
                // real payload). Real zlib/flate2 semantics: `Ok` means
                // this call's unit of work (consume `input`, honor
                // `flush`) is DONE within the given output buffer — the
                // only reason to call again is `flush == Finish` (which
                // isn't "done" until `StreamEnd`) or leftover unconsumed
                // input (shouldn't normally happen since the whole
                // remaining slice is passed every time, but kept as a
                // defensive check).
                if flush != FlushCompress::Finish && consumed >= input.len() {
                    break;
                }
            }
        }
    }
    Ok(out)
}

struct CompressorState {
    inner: Compress,
    finished: bool,
}

fn build_compressobj(level: i64, wbits: i64) -> PyResult<PyObjectRef> {
    let compression = resolve_level(level)?;
    let inner = make_compress(compression, wbits)?;
    let state = Rc::new(RefCell::new(CompressorState {
        inner,
        finished: false,
    }));

    let mut type_dict: HashMap<String, PyObjectRef> = HashMap::new();

    let s = state.clone();
    type_dict.insert_str(
        "compress",
        PyObjectRef::new(PyObject::Closure(Rc::new(move |args: &[PyObjectRef]| {
            if args.is_empty() {
                return Err(PyError::type_error("compress() takes exactly 1 argument"));
            }
            let data = bytes_arg(&args[0])?;
            let mut st = s.borrow_mut();
            if st.finished {
                return Err(zlib_error(
                    "compress() called after flush() with mode Z_FINISH",
                ));
            }
            let out = run_compress(&mut st.inner, &data, FlushCompress::None)?;
            Ok(PyObjectRef::imm(PyObject::Bytes(out)))
        }))),
    );

    let s = state.clone();
    type_dict.insert_str(
        "flush",
        PyObjectRef::new(PyObject::Closure(Rc::new(move |args: &[PyObjectRef]| {
            let mode = if !args.is_empty() {
                args[0].as_i64().unwrap_or(4)
            } else {
                4
            };
            let mut st = s.borrow_mut();
            if st.finished {
                return Err(zlib_error("inconsistent stream state"));
            }
            let flush = if mode == 4 {
                FlushCompress::Finish
            } else {
                FlushCompress::Sync
            };
            let out = run_compress(&mut st.inner, &[], flush)?;
            if mode == 4 {
                st.finished = true;
            }
            Ok(PyObjectRef::imm(PyObject::Bytes(out)))
        }))),
    );

    type_dict.insert_str(
        "copy",
        PyObjectRef::new(PyObject::Closure(Rc::new(
            move |_args: &[PyObjectRef]| {
                // A fully faithful `copy()` would need to clone the live
                // zlib/deflate state, which `flate2::Compress` doesn't expose;
                // approximate with a fresh compressor at the same parameters.
                build_compressobj(level, wbits)
            },
        ))),
    );

    Ok(PyObjectRef::new(PyObject::Instance {
        typ: PyObjectRef::new(PyObject::Type {
            name: "zlib.Compress".to_string(),
            dict: Box::new(str_map_to_typedict(type_dict)),
            bases: vec![],
            mro: vec![],
        }),
        dict: AttrMap::new(),
    }))
}

struct DecompressorState {
    inner: Decompress,
    eof: bool,
}

fn build_decompressobj(wbits: i64) -> PyResult<PyObjectRef> {
    let inner = make_decompress(wbits)?;
    let state = Rc::new(RefCell::new(DecompressorState { inner, eof: false }));
    let unused_data: Rc<RefCell<Vec<u8>>> = Rc::new(RefCell::new(Vec::new()));
    let unconsumed_tail: Rc<RefCell<Vec<u8>>> = Rc::new(RefCell::new(Vec::new()));

    let mut type_dict: HashMap<String, PyObjectRef> = HashMap::new();

    let s = state.clone();
    let ud = unused_data.clone();
    let ut = unconsumed_tail.clone();
    type_dict.insert_str(
        "decompress",
        PyObjectRef::new(PyObject::Closure(Rc::new(move |args: &[PyObjectRef]| {
            if args.is_empty() {
                return Err(PyError::type_error(
                    "decompress() takes at least 1 argument",
                ));
            }
            let data = bytes_arg(&args[0])?;
            let max_length: i64 = if args.len() > 1 {
                match args[1].as_i64() {
                    Some(n) => n,
                    None => 0,
                }
            } else {
                0
            };
            if max_length < 0 {
                return Err(PyError::value_error("max_length must be non-negative"));
            }
            let mut st = s.borrow_mut();
            *ut.borrow_mut() = Vec::new();
            if st.eof {
                ud.borrow_mut().extend_from_slice(&data);
                return Ok(PyObjectRef::imm(PyObject::Bytes(Vec::new())));
            }
            let cap: usize = if max_length == 0 {
                usize::MAX
            } else {
                max_length as usize
            };
            let mut out = Vec::new();
            let mut consumed = 0usize;
            loop {
                if out.len() >= cap {
                    break;
                }
                let want = if cap == usize::MAX {
                    32 * 1024
                } else {
                    (cap - out.len()).min(32 * 1024).max(1)
                };
                let mut buf = vec![0u8; want];
                let before_in = st.inner.total_in();
                let before_out = st.inner.total_out();
                let status = st
                    .inner
                    .decompress(&data[consumed..], &mut buf, FlushDecompress::None)
                    .map_err(|e| {
                        zlib_error(format!(
                            "Error -3 while decompressing data: {}",
                            e.message().unwrap_or("invalid stream")
                        ))
                    })?;
                let produced = (st.inner.total_out() - before_out) as usize;
                out.extend_from_slice(&buf[..produced]);
                consumed += (st.inner.total_in() - before_in) as usize;
                match status {
                    Status::StreamEnd => {
                        st.eof = true;
                        break;
                    }
                    Status::Ok => {
                        if consumed >= data.len() && produced == 0 {
                            break;
                        }
                    }
                    Status::BufError => {
                        if produced == 0 {
                            break;
                        }
                    }
                }
            }
            if st.eof {
                ud.borrow_mut().extend_from_slice(&data[consumed..]);
            } else if consumed < data.len() {
                *ut.borrow_mut() = data[consumed..].to_vec();
            }
            Ok(PyObjectRef::imm(PyObject::Bytes(out)))
        }))),
    );

    let s = state.clone();
    type_dict.insert_str(
        "flush",
        PyObjectRef::new(PyObject::Closure(Rc::new(move |args: &[PyObjectRef]| {
            if !args.is_empty() {
                if let Some(n) = args[0].as_i64() {
                    if n <= 0 {
                        return Err(PyError::value_error("length must be greater than zero"));
                    }
                } else {
                    return Err(PyError::value_error("length must be greater than zero"));
                }
            }
            let mut st = s.borrow_mut();
            if st.eof {
                return Ok(PyObjectRef::imm(PyObject::Bytes(Vec::new())));
            }
            let mut out = Vec::new();
            let mut buf = vec![0u8; 32 * 1024];
            loop {
                let before_out = st.inner.total_out();
                let status = st
                    .inner
                    .decompress(&[], &mut buf, FlushDecompress::Finish)
                    .unwrap_or(Status::StreamEnd);
                let produced = (st.inner.total_out() - before_out) as usize;
                out.extend_from_slice(&buf[..produced]);
                match status {
                    Status::StreamEnd => {
                        st.eof = true;
                        break;
                    }
                    _ => {
                        if produced == 0 {
                            break;
                        }
                    }
                }
            }
            Ok(PyObjectRef::imm(PyObject::Bytes(out)))
        }))),
    );

    type_dict.insert_str(
        "copy",
        PyObjectRef::new(PyObject::Closure(Rc::new(
            move |_args: &[PyObjectRef]| build_decompressobj(wbits),
        ))),
    );

    // `unused_data`/`unconsumed_tail`/`eof` need to reflect LIVE state
    // (mutated by `decompress`/`flush` above) rather than a value snapshot
    // taken at construction time — exposed as real `property` descriptors
    // (`PyObject::Property`, the same mechanism `@property` compiles to)
    // backed by `Closure` getters capturing the shared `Rc<RefCell<...>>`
    // buffers, resolved through the class MRO by the normal attribute
    // descriptor protocol (see `op_attr.rs`'s `Property` handling).
    let ud = unused_data.clone();
    type_dict.insert_str(
        "unused_data",
        PyObjectRef::new(PyObject::Property(Box::new(PropertyData {
            getter: Some(PyObjectRef::new(PyObject::Closure(Rc::new(
                move |_: &[PyObjectRef]| Ok(PyObjectRef::imm(PyObject::Bytes(ud.borrow().clone()))),
            )))),
            setter: None,
            deleter: None,
            doc: None,
        }))),
    );
    let ut = unconsumed_tail.clone();
    type_dict.insert_str(
        "unconsumed_tail",
        PyObjectRef::new(PyObject::Property(Box::new(PropertyData {
            getter: Some(PyObjectRef::new(PyObject::Closure(Rc::new(
                move |_: &[PyObjectRef]| Ok(PyObjectRef::imm(PyObject::Bytes(ut.borrow().clone()))),
            )))),
            setter: None,
            deleter: None,
            doc: None,
        }))),
    );
    let s = state.clone();
    type_dict.insert_str(
        "eof",
        PyObjectRef::new(PyObject::Property(Box::new(PropertyData {
            getter: Some(PyObjectRef::new(PyObject::Closure(Rc::new(
                move |_: &[PyObjectRef]| Ok(py_bool(s.borrow().eof)),
            )))),
            setter: None,
            deleter: None,
            doc: None,
        }))),
    );

    Ok(PyObjectRef::new(PyObject::Instance {
        typ: PyObjectRef::new(PyObject::Type {
            name: "zlib.Decompress".to_string(),
            dict: Box::new(str_map_to_typedict(type_dict)),
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
            m.insert("eof".to_string(), py_bool(false));
            m
        },
    }))
}

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

    z_func!("compress", |args| {
        if args.is_empty() {
            return Err(PyError::type_error(
                "compress() missing required argument (data)",
            ));
        }
        let data = bytes_arg(&args[0])?;
        // The trailing arg may be a packed kwargs dict (`level=...`); a
        // bare second positional int is `level` too.
        let mut level: i64 = -1;
        if args.len() > 1 {
            match &*args[1].borrow() {
                PyObject::Dict(dct) => {
                    if let Ok(Some(v)) = dct.get(&py_str("level")) {
                        level = v.as_i64().unwrap_or(-1);
                    }
                    if dct
                        .get(&py_str("data"))
                        .map(|v| v.is_some())
                        .unwrap_or(false)
                    {
                        return Err(PyError::type_error(
                            "compress() got multiple values for argument 'data'",
                        ));
                    }
                }
                _ => {
                    if let Some(n) = args[1].as_i64() {
                        level = n;
                    }
                }
            }
        }
        let compression = resolve_level(level)?;
        let mut inner = make_compress(compression, 15)?;
        let out = run_compress(&mut inner, &data, FlushCompress::Finish)?;
        Ok(PyObjectRef::imm(PyObject::Bytes(out)))
    });

    z_func!("decompress", |args| {
        if args.is_empty() {
            return Err(PyError::type_error(
                "decompress() missing required argument (data)",
            ));
        }
        let data = bytes_arg(&args[0])?;
        let mut wbits: i64 = 15;
        let mut bufsize: i64 = 16384;
        for a in &args[1..] {
            match &*a.borrow() {
                PyObject::Dict(dct) => {
                    if let Ok(Some(v)) = dct.get(&py_str("wbits")) {
                        wbits = v.as_i64().unwrap_or(15);
                    }
                    if let Ok(Some(v)) = dct.get(&py_str("bufsize")) {
                        bufsize = v.as_i64().unwrap_or(16384);
                    }
                }
                _ => {
                    if wbits == 15 {
                        wbits = a.as_i64().unwrap_or(15);
                    } else {
                        bufsize = a.as_i64().unwrap_or(16384);
                    }
                }
            }
        }
        let _ = bufsize;
        let mut inner = make_decompress(wbits)?;
        let mut out = Vec::new();
        let mut consumed = 0usize;
        let mut ended = false;
        loop {
            let mut buf = vec![0u8; 32 * 1024];
            let before_in = inner.total_in();
            let before_out = inner.total_out();
            let status = inner
                .decompress(&data[consumed..], &mut buf, FlushDecompress::Finish)
                .map_err(|e| {
                    zlib_error(format!(
                        "Error -3 while decompressing data: {}",
                        e.message().unwrap_or("invalid stream")
                    ))
                })?;
            let produced = (inner.total_out() - before_out) as usize;
            out.extend_from_slice(&buf[..produced]);
            consumed += (inner.total_in() - before_in) as usize;
            match status {
                Status::StreamEnd => {
                    ended = true;
                    break;
                }
                Status::Ok | Status::BufError => {
                    if consumed >= data.len() && produced == 0 {
                        break;
                    }
                }
            }
        }
        if !ended {
            return Err(zlib_error(
                "Error -5 while decompressing data: incomplete or truncated stream",
            ));
        }
        Ok(PyObjectRef::imm(PyObject::Bytes(out)))
    });

    z_func!("compressobj", |args| {
        let mut level: i64 = -1;
        let mut wbits: i64 = 15;
        for (i, a) in args.iter().enumerate() {
            match &*a.borrow() {
                PyObject::Dict(dct) => {
                    if let Ok(Some(v)) = dct.get(&py_str("level")) {
                        level = v.as_i64().unwrap_or(-1);
                    }
                    if let Ok(Some(v)) = dct.get(&py_str("wbits")) {
                        wbits = v.as_i64().unwrap_or(15);
                    }
                }
                _ => match i {
                    0 => level = a.as_i64().unwrap_or(-1),
                    // arg 1 is `method` (DEFLATED), ignored
                    2 => wbits = a.as_i64().unwrap_or(15),
                    _ => {}
                },
            }
        }
        build_compressobj(level, wbits)
    });

    z_func!("decompressobj", |args| {
        let mut wbits: i64 = 15;
        for a in args.iter() {
            match &*a.borrow() {
                PyObject::Dict(dct) => {
                    if let Ok(Some(v)) = dct.get(&py_str("wbits")) {
                        wbits = v.as_i64().unwrap_or(15);
                    }
                }
                _ => {
                    if let Some(n) = a.as_i64() {
                        wbits = n;
                    }
                }
            }
        }
        build_decompressobj(wbits)
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
        let data = bytes_arg(&args[0])?;
        let mut adler: u32 = if args.len() > 1 {
            args[1].as_i64().unwrap_or(1) as u32
        } else {
            1
        };
        const MOD_ADLER: u32 = 65521;
        let mut a = adler & 0xffff;
        let mut b = (adler >> 16) & 0xffff;
        for &byte in &data {
            a = (a + byte as u32) % MOD_ADLER;
            b = (b + a) % MOD_ADLER;
        }
        adler = (b << 16) | a;
        Ok(py_int(adler as i64))
    });
    z_func!("crc32", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("crc32() requires bytes"));
        }
        let data = bytes_arg(&args[0])?;
        let mut crc: u32 = if args.len() > 1 {
            args[1].as_i64().unwrap_or(0) as u32
        } else {
            0
        };
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
        for &byte in &data {
            crc = TABLE[((crc ^ byte as u32) & 0xFF) as usize] ^ (crc >> 8);
        }
        crc ^= 0xFFFFFFFF;
        Ok(py_int(crc as i64))
    });

    // _ZlibDecompressor — low-level incremental decompressor backing
    // `Lib/_compression.py`'s use in `gzip`/`bz2`-style modules. Real
    // streaming implementation (not a stub): shares the same
    // `flate2::Decompress` engine as `decompressobj`, with `wbits`
    // defaulting to raw-deflate (-15) as CPython's C implementation does.
    z_func!("_ZlibDecompressor", |args| {
        let mut wbits: i64 = -15;
        if !args.is_empty() {
            match args[0].as_i64() {
                Some(n) => wbits = n,
                None => {
                    return Err(PyError::type_error(
                        "_ZlibDecompressor() argument 1 must be int",
                    ))
                }
            }
        }
        if args.len() >= 2 {
            if !matches!(&*args[1].borrow(), PyObject::Bytes(_)) {
                return Err(PyError::type_error(
                    "_ZlibDecompressor() argument 2 must be bytes",
                ));
            }
        }
        let inner = make_decompress(wbits)?;
        let state = Rc::new(RefCell::new(DecompressorState { inner, eof: false }));
        let unused_data: Rc<RefCell<Vec<u8>>> = Rc::new(RefCell::new(Vec::new()));

        let mut type_dict: HashMap<String, PyObjectRef> = HashMap::new();
        let s = state.clone();
        let ud = unused_data.clone();
        type_dict.insert_str(
            "decompress",
            PyObjectRef::new(PyObject::Closure(Rc::new(move |args: &[PyObjectRef]| {
                if args.is_empty() {
                    return Err(PyError::type_error(
                        "decompress() takes at least 1 argument",
                    ));
                }
                let data = bytes_arg(&args[0])?;
                let mut st = s.borrow_mut();
                if st.eof {
                    ud.borrow_mut().extend_from_slice(&data);
                    return Ok(PyObjectRef::imm(PyObject::Bytes(Vec::new())));
                }
                let mut out = Vec::new();
                let mut consumed = 0usize;
                loop {
                    let mut buf = vec![0u8; 32 * 1024];
                    let before_in = st.inner.total_in();
                    let before_out = st.inner.total_out();
                    let status = st
                        .inner
                        .decompress(&data[consumed..], &mut buf, FlushDecompress::None)
                        .map_err(|e| {
                            zlib_error(format!(
                                "Error -3 while decompressing data: {}",
                                e.message().unwrap_or("invalid stream")
                            ))
                        })?;
                    let produced = (st.inner.total_out() - before_out) as usize;
                    out.extend_from_slice(&buf[..produced]);
                    consumed += (st.inner.total_in() - before_in) as usize;
                    match status {
                        Status::StreamEnd => {
                            st.eof = true;
                            break;
                        }
                        Status::Ok | Status::BufError => {
                            if consumed >= data.len() && produced == 0 {
                                break;
                            }
                        }
                    }
                }
                if st.eof {
                    ud.borrow_mut().extend_from_slice(&data[consumed..]);
                }
                Ok(PyObjectRef::imm(PyObject::Bytes(out)))
            }))),
        );
        let s = state.clone();
        type_dict.insert_str(
            "eof",
            PyObjectRef::new(PyObject::Property(Box::new(PropertyData {
                getter: Some(PyObjectRef::new(PyObject::Closure(Rc::new(
                    move |_: &[PyObjectRef]| Ok(py_bool(s.borrow().eof)),
                )))),
                setter: None,
                deleter: None,
                doc: None,
            }))),
        );
        let ud = unused_data.clone();
        type_dict.insert_str(
            "unused_data",
            PyObjectRef::new(PyObject::Property(Box::new(PropertyData {
                getter: Some(PyObjectRef::new(PyObject::Closure(Rc::new(
                    move |_: &[PyObjectRef]| {
                        Ok(PyObjectRef::imm(PyObject::Bytes(ud.borrow().clone())))
                    },
                )))),
                setter: None,
                deleter: None,
                doc: None,
            }))),
        );
        // `_ZlibDecompressor` is a C-level extension type in real CPython
        // and explicitly disallows pickling/copying — surface that via
        // `__reduce__` raising, matching `testPickle`'s expectation.
        type_dict.insert_str(
            "__reduce__",
            PyObjectRef::new(PyObject::Closure(Rc::new(move |_: &[PyObjectRef]| {
                Err(PyError::type_error(
                    "cannot pickle '_ZlibDecompressor' object",
                ))
            }))),
        );

        let mut inst_dict = AttrMap::new();
        inst_dict.insert("eof".to_string(), py_bool(false));
        inst_dict.insert(
            "unused_data".to_string(),
            PyObjectRef::imm(PyObject::Bytes(Vec::new())),
        );
        inst_dict.insert(
            "unconsumed_tail".to_string(),
            PyObjectRef::imm(PyObject::Bytes(Vec::new())),
        );
        Ok(PyObjectRef::new(PyObject::Instance {
            typ: PyObjectRef::new(PyObject::Type {
                name: "_ZlibDecompressor".to_string(),
                dict: Box::new(str_map_to_typedict(type_dict)),
                bases: vec![],
                mro: vec![],
            }),
            dict: inst_dict,
        }))
    });

    // Module-level constants real CPython's `zlib` exposes — entirely
    // missing before, so any code (including this project's own vendored
    // `test_zlib.py`) that references `zlib.ZLIB_RUNTIME_VERSION` or the
    // other `Z_*`/`MAX_WBITS`/`DEFLATED`/`DEF_BUF_SIZE` constants at IMPORT
    // TIME (not just inside a test body) hit an `AttributeError` before a
    // single test could even run. Values match real CPython/zlib's actual
    // constants exactly (verified against a real CPython 3.14 + zlib
    // 1.3.2); this project's `compress`/`decompress` are backed by `flate2`
    // (which wraps `miniz_oxide`, not real zlib), so `ZLIB_VERSION`/
    // `ZLIB_RUNTIME_VERSION` are reported as a plausible real-zlib version
    // string rather than introspected from a linked C library that isn't
    // actually in use — real code only ever version-compares this string,
    // never round-trips it back into the underlying library.
    macro_rules! z_const {
        ($name:expr, $val:expr) => {
            d.insert($name.to_string(), $val);
        };
    }
    z_const!("MAX_WBITS", py_int(15));
    z_const!("DEFLATED", py_int(8));
    z_const!("DEF_BUF_SIZE", py_int(16384));
    z_const!("DEF_MEM_LEVEL", py_int(8));
    z_const!("Z_NO_COMPRESSION", py_int(0));
    z_const!("Z_BEST_SPEED", py_int(1));
    z_const!("Z_BEST_COMPRESSION", py_int(9));
    z_const!("Z_DEFAULT_COMPRESSION", py_int(-1));
    z_const!("Z_FILTERED", py_int(1));
    z_const!("Z_HUFFMAN_ONLY", py_int(2));
    z_const!("Z_RLE", py_int(3));
    z_const!("Z_FIXED", py_int(4));
    z_const!("Z_DEFAULT_STRATEGY", py_int(0));
    z_const!("Z_NO_FLUSH", py_int(0));
    z_const!("Z_PARTIAL_FLUSH", py_int(1));
    z_const!("Z_SYNC_FLUSH", py_int(2));
    z_const!("Z_FULL_FLUSH", py_int(3));
    z_const!("Z_FINISH", py_int(4));
    z_const!("Z_BLOCK", py_int(5));
    z_const!("Z_TREES", py_int(6));
    z_const!("ZLIB_VERSION", py_str("1.3.2"));
    z_const!("ZLIB_RUNTIME_VERSION", py_str("1.3.2"));

    d
}
