// Helpers extracted from src/object/attrs/mod.rs — sequence repetition,
// encoding, deque mutation guards, float utilities, etc.
use crate::object::*;
use num_bigint::BigInt;
use num_traits::Zero;

/// Lowercase with CPython's GREEK FINAL SIGMA special-casing: a lowercase
/// sigma (U+03C3) that ends a word (a cased char before, no cased char
/// after) becomes final sigma (U+03C2). Rust's plain `to_lowercase()`
/// always yields U+03C3, but CPython's str.lower/swapcase/title/capitalize
/// respect the Final_Sigma context rule (`'A\u03a3'.lower() == 'a\u03c2'`).
pub(crate) fn lower_with_final_sigma(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    for (i, &c) in chars.iter().enumerate() {
        let prev_cased = i > 0 && (chars[i - 1].is_uppercase() || chars[i - 1].is_lowercase());
        for lc in c.to_lowercase() {
            if lc == '\u{03C3}' {
                let next_cased = i + 1 < chars.len()
                    && (chars[i + 1].is_uppercase() || chars[i + 1].is_lowercase());
                if prev_cased && !next_cased {
                    out.push('\u{03C2}');
                } else {
                    out.push('\u{03C3}');
                }
            } else {
                out.push(lc);
            }
        }
    }
    out
}

/// Convert an f64 to its exact integer via ceil/floor/trunc (`mode`: 0 =
/// trunc toward zero, 1 = ceil toward +inf, 2 = floor toward -inf). Returns
/// an error for nan/inf. Handles values beyond i64 range (1.23e167).
pub(crate) fn f64_to_int_ceil_floor_trunc(v: f64, mode: u8) -> PyResult<BigInt> {
    if v.is_nan() {
        return Err(PyError::value_error("cannot convert float NaN to integer"));
    }
    if v.is_infinite() {
        return Err(PyError::overflow_error(
            "cannot convert float infinity to integer",
        ));
    }
    let bits = v.to_bits();
    let sign = if (bits >> 63) == 0 { 1 } else { -1 };
    let biased = ((bits >> 52) & 0x7ff) as i32;
    let mantissa = bits & 0x000f_ffff_ffff_ffff;
    if v == 0.0 {
        return Ok(BigInt::from(0));
    }
    let exp = biased - 1023;
    let mantissa_full = if biased == 0 {
        mantissa
    } else {
        (1u64 << 52) | mantissa
    };
    let mag = BigInt::from(mantissa_full);
    // value = mantissa_full * 2^(exp - 52) — the 52-bit fraction offset.
    // q = trunc(|v|), had_frac = whether |v| is non-integral.
    let (q, had_frac) = if exp >= 52 {
        (mag << ((exp - 52) as usize), false)
    } else {
        let shift = (52 - exp) as usize;
        let denom = BigInt::from(1u64) << shift;
        let q = &mag / &denom;
        let r = &mag % &denom;
        (q, !r.is_zero())
    };
    if sign < 0 {
        // trunc(-x) = -q; ceil(-x) = -q (toward +inf); floor(-x) = -(q+1)
        // if fractional.
        return Ok(match mode {
            0 => -q,
            1 => -q,
            _ => {
                if had_frac {
                    -q - 1
                } else {
                    -q
                }
            }
        });
    }
    Ok(match mode {
        0 => q,
        1 => {
            if had_frac {
                q + 1
            } else {
                q
            }
        }
        _ => q,
    })
}

pub fn float_binop_dunder(args: &[PyObjectRef], reverse: bool, kind: u8) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Err(PyError::type_error("binary operator needs 2 arguments"));
    }
    // The established native-dunder calling convention: reached via the
    // operator path (`try_dunder_binop`) the closure receives
    // `[self_obj, self, other]` (3 args), but a DIRECT call like
    // `(1.0).__truediv__(2)` delivers `[self, other]` (2 args, the bound
    // self already consumed). Reading the LAST TWO args handles both.
    let (a, b) = (&args[args.len() - 2], &args[args.len() - 1]);
    let (a, b) = if reverse { (b, a) } else { (a, b) };
    let af = a.as_f64();
    let bf = b.as_f64();
    // A non-numeric operand (e.g. a custom class implementing __float__)
    // must yield NotImplemented so the operator machinery falls through to
    // the other side's reflected dunder (`1.0 + Rat` -> Rat.__radd__) —
    // computing with NAN would swallow the real result. Note as_f64 returns
    // Some for a REAL float('nan'), so genuine NaN still propagates.
    if af.is_none() || bf.is_none() {
        return Ok(crate::object::py_not_implemented());
    }
    let af = af.unwrap();
    let bf = bf.unwrap();
    // `0.0 ** negative` must raise ZeroDivisionError ("0.0 cannot be
    // raised to a negative power") — powf would return inf (test_pow's
    // test_powfloat asserts the error). But 0.0 ** -inf legitimately
    // diverges to inf (non-finite exponent).
    if kind == 3 && af == 0.0 && bf < 0.0 && bf.is_finite() {
        return Err(PyError::zero_division());
    }
    let result = match kind {
        0 => af / bf,
        1 => (af / bf).floor(),
        2 => {
            // Python's %: result has the divisor's sign; a zero result takes
            // the divisor's sign too (-0.0 % 1.0 == 0.0).
            let rem = af % bf;
            if rem == 0.0 {
                if bf.is_sign_negative() {
                    -0.0
                } else {
                    0.0
                }
            } else if (rem < 0.0) != (bf < 0.0) {
                rem + bf
            } else {
                rem
            }
        }
        3 => {
            // Negative base with a non-integer exponent defers to complex
            // pow ((-2.0)**0.5 is complex); 0.0**negative raises. -INF stays
            // on the real path (powf(-inf, -0.5) == 0.0).
            if af < 0.0 && af.is_finite() && bf.fract() != 0.0 && bf.is_finite() {
                let r = (-af).powf(bf);
                let theta = bf * std::f64::consts::PI;
                return Ok(PyObjectRef::imm(PyObject::Complex(
                    r * theta.cos(),
                    r * theta.sin(),
                )));
            }
            af.powf(bf)
        }
        4 => af + bf,
        5 => af - bf,
        6 => af * bf,
        _ => return Err(PyError::type_error("unknown operator")),
    };
    Ok(py_float(result))
}

/// Snapshot a deque's items plus its length at entry, releasing the borrow
/// — used by `count`/`index`/`remove`/`__contains__` so a hostile element
/// `__eq__` can reenter and mutate the deque mid-comparison without a
/// `RefCell` conflict, while still letting the caller detect the mutation
/// afterward (CPython raises `RuntimeError: deque mutated during
/// iteration` when the length changes during one of these).
pub(crate) fn snapshot_deque(obj: &PyObjectRef) -> PyResult<(Vec<PyObjectRef>, usize)> {
    let borrowed = obj.borrow();
    if let PyObject::Deque { data, .. } = &*borrowed {
        Ok((data.iter().cloned().collect(), data.len()))
    } else {
        Err(PyError::runtime_error("not a deque"))
    }
}

/// Raise if the deque's length changed since `start_len` (i.e. some
/// element's `__eq__` mutated it mid-operation). CPython raises DIFFERENT
/// error types per method: `RuntimeError: deque mutated during iteration`
/// for `contains`/`count`/`index`, but `IndexError: deque mutated during
/// iteration` for `remove` (see `deque_remove` in CPython's
/// `collectionsmodule.c`).
pub(crate) fn check_deque_not_mutated(obj: &PyObjectRef, start_len: usize, err: &'static str) -> PyResult<()> {
    let borrowed = obj.borrow();
    if let PyObject::Deque { data, .. } = &*borrowed {
        if data.len() != start_len {
            return Err(if err == "index" {
                PyError::index_error("deque mutated during iteration")
            } else {
                PyError::runtime_error("deque mutated during iteration")
            });
        }
    }
    Ok(())
}

/// Rich `==` between two elements, routed through `py_compare` so a custom
/// `Instance` element's `__eq__` is consulted (deque methods use this rather
/// than the lower-level `PyObject::equals`, which for e.g. `int == Instance`
/// doesn't invoke the instance's reflected `__eq__` — real trigger:
/// `seq_tests.CommonTest`'s `ALWAYS_EQ`/`NEVER_EQ` checks, which rely on the
/// instance dunder being called for `deque.count`/`index`/`contains`).
pub(crate) fn deque_rich_eq(a: &PyObjectRef, b: &PyObjectRef) -> PyResult<bool> {
    py_compare(a, b, 2).map(|r| r.truthy())
}

/// `str.encode(encoding='utf-8', errors='strict')` — was completely
/// ignoring `encoding` and always emitting raw UTF-8 bytes regardless of
/// what was actually requested, so `"Ç".encode("latin-1")` silently
/// returned the UTF-8 bytes `b'\xc3\x87'` instead of the correct
/// single-byte `b'\xc7'`. Confirmed via `test_utf8source.py::test_latin1`
/// (round-tripping non-ASCII source text through `.encode("Latin-1")` to
/// build a `bytes` source for `compile()`). Handles the common
/// single-byte encodings directly; anything else still falls back to
/// UTF-8 (matching the previous, universal behavior) rather than raising,
/// to avoid regressing any caller that never specifies a real encoding.
pub(crate) fn str_encode_builtin(a: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let s = a[0].str();
    let encoding = if a.len() > 1 && !matches!(&*a[1].borrow(), PyObject::None) {
        a[1].str()
    } else {
        "utf-8".to_string()
    };
    let errors = if a.len() > 2 && !matches!(&*a[2].borrow(), PyObject::None) {
        a[2].str()
    } else {
        "strict".to_string()
    };
    let norm = encoding.to_ascii_lowercase().replace('_', "-");
    let norm2 = norm.replace('-', "_");
    // utf-8 family - handle directly
    if norm == "utf-8" || norm == "utf8" || norm2 == "utf_8" || norm2 == "utf8" {
        // For utf-8 we still need to respect errors for surrogates? just return bytes
        return Ok(PyObjectRef::imm(PyObject::Bytes(s.into_bytes())));
    }
    let bytes = match norm.as_str() {
        "latin-1" | "latin1" | "iso-8859-1" | "iso8859-1" | "l1" | "8859" | "cp819" => {
            let mut out = Vec::with_capacity(s.len());
            for (i, c) in s.chars().enumerate() {
                let cp = c as u32;
                if cp > 0xFF {
                    // surrogateescape: lone surrogates DC80-DCFF map back to bytes 80-FF
                    if errors == "surrogateescape" && (0xDC80..=0xDCFF).contains(&cp) {
                        out.push((cp - 0xDC00) as u8);
                        continue;
                    }
                    if errors == "strict" {
                        return Err(PyError::Exception(
                            "UnicodeEncodeError".to_string(),
                            PyObjectRef::new(PyObject::Exception {
                                typ: "UnicodeEncodeError".to_string(),
                                args: vec![
                                    py_str(&encoding),
                                    py_str(&s),
                                    py_int(i as i64),
                                    py_int(i as i64 + 1),
                                    py_str("ordinal not in range(256)"),
                                ],
                                cause: None,
                                suppress_context: false,
                                context: None,
                                traceback: None,
                                extra: None,
                            }),
                        ));
                    } else if errors == "ignore" {
                        continue;
                    } else if errors == "replace" {
                        out.push(b'?');
                        continue;
                    } else if errors == "xmlcharrefreplace" {
                        let esc = format!("&#{};", cp);
                        out.extend_from_slice(esc.as_bytes());
                        continue;
                    } else if errors == "backslashreplace" || errors == "namereplace" {
                        let esc = if cp < 0x100 {
                            format!("\\x{:02x}", cp)
                        } else if cp < 0x10000 {
                            format!("\\u{:04x}", cp)
                        } else {
                            format!("\\U{:08x}", cp)
                        };
                        out.extend_from_slice(esc.as_bytes());
                        continue;
                    } else if errors == "surrogateescape" {
                        return Err(PyError::Exception(
                            "UnicodeEncodeError".to_string(),
                            PyObjectRef::new(PyObject::Exception {
                                typ: "UnicodeEncodeError".to_string(),
                                args: vec![
                                    py_str(&encoding),
                                    py_str(&s),
                                    py_int(i as i64),
                                    py_int(i as i64 + 1),
                                    py_str("ordinal not in range(256)"),
                                ],
                                cause: None,
                                suppress_context: false,
                                context: None,
                                traceback: None,
                                extra: None,
                            }),
                        ));
                    } else {
                        return Err(PyError::Exception(
                            "UnicodeEncodeError".to_string(),
                            PyObjectRef::new(PyObject::Exception {
                                typ: "UnicodeEncodeError".to_string(),
                                args: vec![
                                    py_str(&encoding),
                                    py_str(&s),
                                    py_int(i as i64),
                                    py_int(i as i64 + 1),
                                    py_str("ordinal not in range(256)"),
                                ],
                                cause: None,
                                suppress_context: false,
                                context: None,
                                traceback: None,
                                extra: None,
                            }),
                        ));
                    }
                }
                out.push(cp as u8);
            }
            out
        }
        "ascii" | "us-ascii" | "646" => {
            let mut out = Vec::with_capacity(s.len());
            for (i, c) in s.chars().enumerate() {
                let cp = c as u32;
                if cp > 0x7F {
                    // surrogateescape: surrogates DC80-DCFF map to bytes 80-FF, also handle direct 0x80-0xFF for latin-1 style decode
                    if errors == "surrogateescape" && (0xDC80..=0xDCFF).contains(&cp) {
                        out.push((cp - 0xDC00) as u8);
                        continue;
                    }
                    if errors == "surrogateescape" && (0x80..=0xFF).contains(&cp) {
                        out.push(cp as u8);
                        continue;
                    }
                    if errors == "strict" {
                        return Err(PyError::Exception(
                            "UnicodeEncodeError".to_string(),
                            PyObjectRef::new(PyObject::Exception {
                                typ: "UnicodeEncodeError".to_string(),
                                args: vec![
                                    py_str(&encoding),
                                    py_str(&s),
                                    py_int(i as i64),
                                    py_int(i as i64 + 1),
                                    py_str("ordinal not in range(128)"),
                                ],
                                cause: None,
                                suppress_context: false,
                                context: None,
                                traceback: None,
                                extra: None,
                            }),
                        ));
                    } else if errors == "ignore" {
                        continue;
                    } else if errors == "replace" {
                        out.push(b'?');
                        continue;
                    } else if errors == "xmlcharrefreplace" {
                        let esc = format!("&#{};", cp);
                        out.extend_from_slice(esc.as_bytes());
                        continue;
                    } else if errors == "backslashreplace" {
                        let esc = if cp < 0x100 {
                            format!("\\x{:02x}", cp)
                        } else if cp < 0x10000 {
                            format!("\\u{:04x}", cp)
                        } else {
                            format!("\\U{:08x}", cp)
                        };
                        out.extend_from_slice(esc.as_bytes());
                        continue;
                    } else if errors == "namereplace" {
                        let esc = if cp < 0x100 {
                            format!("\\x{:02x}", cp)
                        } else if cp < 0x10000 {
                            format!("\\u{:04x}", cp)
                        } else {
                            format!("\\U{:08x}", cp)
                        };
                        out.extend_from_slice(esc.as_bytes());
                        continue;
                    } else if errors == "surrogateescape" {
                        return Err(PyError::Exception(
                            "UnicodeEncodeError".to_string(),
                            PyObjectRef::new(PyObject::Exception {
                                typ: "UnicodeEncodeError".to_string(),
                                args: vec![
                                    py_str(&encoding),
                                    py_str(&s),
                                    py_int(i as i64),
                                    py_int(i as i64 + 1),
                                    py_str("ordinal not in range(128)"),
                                ],
                                cause: None,
                                suppress_context: false,
                                context: None,
                                traceback: None,
                                extra: None,
                            }),
                        ));
                    } else {
                        return Err(PyError::Exception(
                            "UnicodeEncodeError".to_string(),
                            PyObjectRef::new(PyObject::Exception {
                                typ: "UnicodeEncodeError".to_string(),
                                args: vec![
                                    py_str(&encoding),
                                    py_str(&s),
                                    py_int(i as i64),
                                    py_int(i as i64 + 1),
                                    py_str("ordinal not in range(128)"),
                                ],
                                cause: None,
                                suppress_context: false,
                                context: None,
                                traceback: None,
                                extra: None,
                            }),
                        ));
                    }
                }
                out.push(cp as u8);
            }
            out
        }
        _ => {
            // Try generic codec lookup (e.g. testcodec)
            if let Some(codec_tuple) = crate::modules::lookup_codec(&encoding) {
                let encode_fn = {
                    let tup = codec_tuple.borrow();
                    if let PyObject::Tuple(items) = &*tup {
                        items.get(0).cloned()
                    } else {
                        match tup.get_attribute("encode") {
                            Ok(v) => Some(v),
                            Err(_) => None,
                        }
                    }
                };
                if let Some(ef) = encode_fn {
                    let str_obj = a[0].clone();
                    match crate::object::call_function_disposable(&ef, vec![str_obj, py_str(&errors)], vec![]) {
                        Ok(res) => {
                            let bytes_obj = {
                                let br = res.borrow();
                                if let PyObject::Tuple(items) = &*br {
                                    if !items.is_empty() {
                                        items[0].clone()
                                    } else {
                                        res.clone()
                                    }
                                } else {
                                    res.clone()
                                }
                            };
                            let b = bytes_obj.borrow();
                            if let PyObject::Bytes(v) = &*b {
                                return Ok(PyObjectRef::imm(PyObject::Bytes(v.clone())));
                            } else if let PyObject::ByteArray(v) = &*b {
                                return Ok(PyObjectRef::imm(PyObject::Bytes(v.clone())));
                            } else {
                                // if codec returned str (unlikely), convert
                                return Ok(PyObjectRef::imm(PyObject::Bytes(
                                    bytes_obj.str().as_bytes().to_vec(),
                                )));
                            }
                        }
                        Err(e) => return Err(e),
                    }
                }
            }
            // fallback to utf-8
            s.as_bytes().to_vec()
        },
    };
    Ok(PyObjectRef::imm(PyObject::Bytes(bytes)))
}

/// The integer VALUE of an int or bool object (bool is int's subtype).
pub(crate) fn int_or_bool_value(o: &PyObjectRef) -> Option<BigInt> {
    match &*o.borrow() {
        PyObject::Int(i) => Some(i.clone()),
        PyObject::Bool(b) => Some(BigInt::from(*b as i64)),
        _ => None,
    }
}

/// `int.__new__(bool, ...)` is a TypeError (test_bool::test_subclass) —
/// bool has its own allocator; everything else delegates to `int()`.
pub(crate) fn int_new_checked(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if let Some(cls) = args.first() {
        let is_bool = match &*cls.borrow() {
            PyObject::Bool(_) => true,
            PyObject::Type { name, .. } => name == "bool",
            _ => false,
        };
        if is_bool {
            return Err(PyError::type_error(
                "int.__new__(bool) is not safe, use bool.__new__()",
            ));
        }
    }
    crate::object::builtin_int(args)
}

/// Direct sequence repetition for the `__mul__`/`__rmul__` dunders. NOT via
/// `py_mul` — that would re-dispatch through `try_dunder_binop` back into the
/// same `__mul__` (infinite recursion). Builds the repeated container.
pub(crate) fn dunder_repeat(obj: &PyObjectRef, count: &PyObjectRef) -> PyResult<PyObjectRef> {
    let idx = crate::object::to_index(count)?;
    // A repeat count that can't fit C ssize_t must OverflowError, not be
    // silently clamped (test_index::test_sequence_repeat: 'a' * 2**100).
    // Small negatives are fine: `[1,2] * -1` -> `[]` (test_list::test_repeat).
    let i64_n = idx.to_i64();
    if i64_n.is_none() {
        if idx.sign() == num_bigint::Sign::Minus {
            // magnitude overflows ssize_t
            return Err(PyError::overflow_error("negative count"));
        }
        return Err(PyError::overflow_error("repeated value is too large"));
    }
    let n = i64_n.unwrap().max(0) as usize;
    if n == 1 {
        // `seq * 1` returns the SAME object (CPython's immutable
        // optimization — `id(s) == id(s*1)` for tuples).
        return Ok(obj.clone());
    }
    let borrowed = obj.borrow();
    match &*borrowed {
        PyObject::List(items) => {
            // Fail fast on overflow like list_resize ([0] * sys.maxsize ->
            // MemoryError), never panic on Vec::with_capacity.
            let total = match items.len().checked_mul(n) {
                Some(t) => t,
                None => return Err(PyError::memory_error("could not allocate list")),
            };
            let mut probe: Vec<PyObjectRef> = Vec::new();
            if probe.try_reserve_exact(total).is_err() {
                return Err(PyError::memory_error("could not allocate list"));
            }
            let mut out = Vec::with_capacity(total);
            for _ in 0..n {
                out.extend(items.iter().cloned());
            }
            Ok(py_list(out))
        }
        PyObject::Tuple(items) => {
            let total = match items.len().checked_mul(n) {
                Some(t) => t,
                None => return Err(PyError::memory_error("could not allocate tuple")),
            };
            let mut probe: Vec<PyObjectRef> = Vec::new();
            if probe.try_reserve_exact(total).is_err() {
                return Err(PyError::memory_error("could not allocate tuple"));
            }
            let mut out = Vec::with_capacity(total);
            for _ in 0..n {
                out.extend(items.iter().cloned());
            }
            Ok(py_tuple(out))
        }
        PyObject::Str(s) => match s.len().checked_mul(n) {
            Some(total) => {
                let mut probe: Vec<u8> = Vec::new();
                if probe.try_reserve_exact(total).is_err() {
                    return Err(PyError::memory_error("could not allocate string"));
                }
                Ok(py_str(&s.repeat(n)))
            }
            None => Err(PyError::overflow_error("repeated string is too long")),
        },
        PyObject::Bytes(b) => {
            let total = match b.len().checked_mul(n) {
                Some(t) => t,
                None => return Err(PyError::overflow_error("bytes object is too large")),
            };
            let mut probe: Vec<u8> = Vec::new();
            if probe.try_reserve_exact(total).is_err() {
                return Err(PyError::memory_error("could not allocate bytes"));
            }
            let mut out = Vec::with_capacity(total);
            for _ in 0..n {
                out.extend_from_slice(b);
            }
            Ok(PyObjectRef::imm(PyObject::Bytes(out)))
        }
        PyObject::ByteArray(b) => {
            let total = match b.len().checked_mul(n) {
                Some(t) => t,
                None => return Err(PyError::overflow_error("bytearray object is too large")),
            };
            let mut probe: Vec<u8> = Vec::new();
            if probe.try_reserve_exact(total).is_err() {
                return Err(PyError::memory_error("could not allocate bytearray"));
            }
            let mut out = Vec::with_capacity(total);
            for _ in 0..n {
                out.extend_from_slice(&b[..]);
            }
            Ok(PyObjectRef::imm(PyObject::ByteArray(out)))
        }
        _ => Err(PyError::type_error("sequence type not repeatable")),
    }
}
