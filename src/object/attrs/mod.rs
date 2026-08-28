// Split out of the former monolithic object.rs (see the file-splitting
// refactor's memory entry for context) — this file holds attribute
// access: `get_attribute_impl` (the giant dispatcher backing
// `LOAD_ATTR`/`getattr`/method lookup across every builtin type and
// user-defined class) and its supporting helpers. NOT further broken up
// internally in this pass — see the plan's own note on scope.
use super::*;

mod deque;
mod list;
mod bytes1;
mod bytes2;
mod str1;
mod str2;
mod int;
mod float;
mod compiled_regex;
mod range;
mod tuple;
mod array;
mod frozenset;
mod slice;
mod complex;
mod module_obj;
mod exception_group;
mod generator;
mod set;
mod file;
mod socket;
mod thread;

thread_local! {
    // PEP 649 computed-annotation cache, keyed by each function's
    // `__annotate__` closure identity (see the `__annotations__` arm).
    static ANN_CACHE: std::cell::RefCell<std::collections::HashMap<usize, PyObjectRef>> = std::cell::RefCell::new(std::collections::HashMap::new());
}

/// Binary-op dispatch for float's numeric-protocol dunders
/// (`float.__truediv__`, `__rsub__`, ...). `kind` selects the operator, and
/// `reverse` swaps the operands for the reflected forms. Computes the raw
/// f64 arithmetic DIRECTLY (NOT via py_div/py_add etc., which re-dispatch
/// the dunder and would recurse infinitely). Referenced from non-capturing
/// closures so it coerces to `BuiltinFunc` (fn pointer).
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

// ---- Attribute access ----

pub trait ObjectAccess {
    fn get_attribute(&self, name: &str) -> PyResult<PyObjectRef>;
    fn set_attribute(&mut self, name: &str, value: PyObjectRef) -> PyResult<()>;
    fn del_attribute(&mut self, name: &str) -> PyResult<()>;
}

/// Snapshot a deque's items plus its length at entry, releasing the borrow
/// — used by `count`/`index`/`remove`/`__contains__` so a hostile element
/// `__eq__` can reenter and mutate the deque mid-comparison without a
/// `RefCell` conflict, while still letting the caller detect the mutation
/// afterward (CPython raises `RuntimeError: deque mutated during
/// iteration` when the length changes during one of these).
fn snapshot_deque(obj: &PyObjectRef) -> PyResult<(Vec<PyObjectRef>, usize)> {
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
fn check_deque_not_mutated(obj: &PyObjectRef, start_len: usize, err: &'static str) -> PyResult<()> {
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
fn deque_rich_eq(a: &PyObjectRef, b: &PyObjectRef) -> PyResult<bool> {
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
fn str_encode_builtin(a: &[PyObjectRef]) -> PyResult<PyObjectRef> {
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
fn int_new_checked(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
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
fn dunder_repeat(obj: &PyObjectRef, count: &PyObjectRef) -> PyResult<PyObjectRef> {
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

impl PyObject {
    /// Every real Python object has `__doc__` (defaulting to `None` if not
    /// otherwise set — `bool`/`int`/etc. all inherit it from `object`).
    /// The per-variant match below (a few thousand lines, one arm per
    /// builtin type, each with its own "no such attribute" catch-all) has
    /// no single place to add a universal fallback without touching every
    /// arm — so it stays untouched as `get_attribute_impl`, and the real
    /// `get_attribute` (the trait method below) just catches this one
    /// specific case on error instead. Real code doing generic attribute
    /// introspection over arbitrary values (e.g. something in the stdlib
    /// `email`/`dataclasses` machinery checking `.__doc__` while walking a
    /// structure that isn't guaranteed to be a function/class) hit this.
    pub(crate) fn get_attribute_impl(&self, name: &str) -> PyResult<PyObjectRef> {
        // `.__class__` (equivalent to `type(x)`) universally, for every
        // variant — this was entirely missing from `get_attribute_impl`
        // (used by the `getattr()` builtin and any other generic
        // attribute-access call site), even for a plain `class Foo: ...`
        // instance, even though `x.__class__` (direct dot-syntax) already
        // worked via a separate, hardcoded special case in `vm.rs`'s
        // LOAD_ATTR opcode handler. So `getattr(x, "__class__")` — a common
        // proxy/introspection idiom real code uses interchangeably with
        // `type(x)` — raised `AttributeError` for literally every object,
        // real trigger: CPython 3.14's own `unittest/case.py`
        // (`self.__class__` reached via a code path that goes through
        // `get_attribute_impl` rather than LOAD_ATTR). Mirrors
        // `builtin_type_of`'s own logic (Instance → its real type;
        // Type → itself; anything else → a freshly-built placeholder Type
        // sharing just the name, same as `type(x)` already does for
        // natives).
        // Per-instance attributes stored on `functools.partial` objects
        // (CPython's partial has a real __dict__; configparser assigns
        // `self.converter = ...` on one).
        if !name.starts_with("__") {
            if let PyObject::Partial { dict, .. } = self {
                if let Some(v) = dict.get(name) {
                    return Ok(v.clone());
                }
            }
        }
        if name == "__class__" {
            match self {
                PyObject::Instance { typ, .. } => return Ok(typ.clone()),
                // A class's own `__class__` is its metaclass — usually
                // plain `type`. `metatype_of()` (used elsewhere for the
                // real, `METATYPE_KEY`-tracked custom-metaclass case) needs
                // a `PyObjectRef`, not the bare `&PyObject` available here;
                // falling back to plain `"type"` is correct for the
                // overwhelmingly common no-custom-metaclass case.
                PyObject::Type { dict, .. } if dict.contains_key_str(METATYPE_KEY) => {
                    return Ok(dict.get_str(METATYPE_KEY).unwrap().clone());
                }
                PyObject::Type { .. } => {
                    return Ok(PyObjectRef::new(PyObject::Type {
                        name: "type".to_string(),
                        dict: Box::new(TypeDict::default()),
                        bases: vec![],
                        mro: vec![],
                    }));
                }
                _ => {
                    return Ok(PyObjectRef::new(PyObject::Type {
                        name: self.type_name().to_string(),
                        dict: Box::new(TypeDict::default()),
                        bases: vec![],
                        mro: vec![],
                    }));
                }
            }
        }
        // `.__dir__` — `dir()` itself (`builtin_dir`) already introspects
        // every variant directly and doesn't need this, but `dir()`'s own
        // listing always advertises a synthetic `"__dir__"` name (matching
        // real CPython, where every object inherits `object.__dir__`), and
        // code that walks that listing generically (`getattr(obj, name) for
        // name in dir(obj)` — real trigger: CPython 3.14's own
        // `unittest/loader.py`'s `loadTestsFromModule`) then does
        // `getattr(module, "__dir__")`, which raised `AttributeError` since
        // no variant actually exposed it as a real bindable attribute.
        // Doesn't check for a user-overridden `__dir__` first (unlike a
        // real per-type dict lookup) — a rare enough case in practice that
        // matching the `.__class__` fix's pragmatic same-shape precedent
        // (a universal fallback) is the right tradeoff here.
        if name == "__dir__" {
            return Ok(PyObjectRef::new(PyObject::BuiltinMethod {
                name: "__dir__".to_string(),
                func: builtin_dir,
                self_obj: py_none(),
            }));
        }
        match self {
            PyObject::Complex(_, _) => return complex::get(self, name),
            PyObject::WeakProxy { target, .. } => {
                if let Some(rc) = target.upgrade() {
                    let t = PyObjectRef::Imm(rc);
                    return t.borrow().get_attribute(name);
                } else {
                    return Err(PyError::reference_error("weakly-referenced object no longer exists"));
                }
            }
            PyObject::Module { .. } => return module_obj::get(self, name),
            PyObject::Type {
                dict,
                mro,
                bases,
                name: type_name,
            } => {
                if name == "__dict__" {
                    // Return type's dict as a PyDict — NATIVE_BASE_MARKER is
                    // an internal bookkeeping entry (see native_base_of_type)
                    // and must not leak into user-visible introspection.
                    let mut pd = PyDict::new();
                    for (k, v) in dict.iter() {
                        let k_str = interner::lookup_str(*k);
                        if k_str == NATIVE_BASE_MARKER
                            || k_str == METATYPE_KEY
                            || k_str == NATIVE_VALUE_CTOR_KEY
                        {
                            continue;
                        }
                        let _ = pd.set(py_str(k_str), v.clone());
                    }
                    return Ok(PyObjectRef::new(PyObject::Dict(Box::new(pd))));
                }
                if name == "__mro__" {
                    return Ok(PyObjectRef::new(PyObject::Tuple(mro.clone())));
                }
                if name == "__bases__" {
                    return Ok(PyObjectRef::new(PyObject::Tuple(bases.clone())));
                }
                if name == "__name__" {
                    return Ok(py_str(type_name));
                }
                if name == "__qualname__" {
                    return Ok(py_str(type_name));
                }
                if name == "__annotations__" {
                    if let Some(v) = dict.get_str("__annotations__").cloned() {
                        return Ok(v);
                    }
                    return Ok(crate::object::py_dict());
                }
                // `__module__` — real user-defined classes already have this
                // seeded into their own dict at class-creation time (the
                // class body's implicit `__module__ = __name__` statement),
                // so this fallback is only ever reached for BUILTIN/native
                // ad-hoc types (`int`, `str`, `types.UnionType`, ...), which
                // never went through that seeding. Defaults to `"builtins"`
                // (correct for the real native types; a reasonable filler
                // for ad-hoc "instance-shaped" native types like `Path`/
                // `SimpleNamespace`/`UnionType`, which have no real module
                // of their own to report) — found via CPython's own
                // `test_types.py`'s `check_disallow_instantiation` helper,
                // which unconditionally reads `tp.__module__` on ANY type.
                if name == "__module__" && !dict.contains_key_str("__module__") {
                    // `array`'s instances live in the `array` module —
                    // reprlib's dispatch keys on `type(x).__module__`
                    // (test_reprlib::test_container).
                    if type_name == "array" {
                        return Ok(py_str("array"));
                    }
                    return Ok(py_str("builtins"));
                }
                // PEP 604 union syntax (`int | str`, `MyClass | None`) — the
                // `|` operator was entirely unsupported on ANY class/builtin
                // type (`TypeError: unsupported operand type(s) for |: ...`)
                // even though it's an extremely common modern idiom in type
                // annotations (`def f(x: int | str)`) and isinstance checks
                // (`isinstance(x, int | None)`), evaluated at RUNTIME
                // whenever the annotation isn't behind `from __future__
                // import annotations`. Gated on the type's own dict NOT
                // already defining `__or__`/`__ror__` (same pattern as
                // `register` just above) so a class that genuinely overrides
                // either keeps its own behavior.
                if (name == "__or__" || name == "__ror__") && !dict.contains_key_str(name) {
                    // A plain `BuiltinFunction`, NOT `BuiltinMethod` — the
                    // latter's `call_bound_method` convention prepends an
                    // extra placeholder `self_obj` ahead of `self`/`other`
                    // (3 args: `[None, self, other]`), which silently
                    // shifted every argument here by one (confirmed via a
                    // direct repro: `int | str` built a union of `[None,
                    // int]` instead of `[int, str]`). `BuiltinFunction`'s own
                    // convention is the plain 2-arg `[self, other]` these
                    // closures actually expect — see `try_dunder_binop`'s own
                    // doc comment for the exact convention split between the
                    // two.
                    return Ok(PyObjectRef::imm(PyObject::BuiltinFunction {
                        name: name.to_string(),
                        func: if name == "__or__" {
                            |args| {
                                if args.len() < 2 {
                                    return Err(PyError::type_error("__or__() missing argument"));
                                }
                                Ok(crate::modules::make_union(vec![
                                    args[0].clone(),
                                    args[1].clone(),
                                ]))
                            }
                        } else {
                            |args| {
                                if args.len() < 2 {
                                    return Err(PyError::type_error("__ror__() missing argument"));
                                }
                                Ok(crate::modules::make_union(vec![
                                    args[1].clone(),
                                    args[0].clone(),
                                ]))
                            }
                        },
                    }));
                }
                // `ABCMeta.register(subclass)` — real CPython's `abc.py`
                // wraps a native `_abc_register` primitive that this
                // project already implements (`modules/core.rs`) but never
                // actually wires up: `class Foo(metaclass=ABCMeta): ...`
                // doesn't go through a real `class ABCMeta(type):` (this
                // project's own `ABCMeta` is a plain `BuiltinFunction`, not
                // a `type` subclass — real per-metaclass method lookup
                // falling back from `SomeClass.register` to `type
                // (SomeClass).register` is a deeper, unimplemented
                // architecture piece), so `SomeClass.register` never
                // resolved to anything at all. Providing `.register` as a
                // generic fallback on EVERY class (not gated on "was this
                // built via ABCMeta") is pragmatic rather than fully
                // correct — but calling `.register()` on a non-ABC class
                // isn't something real code does unintentionally, so
                // there's no real-world downside. Records the virtual
                // subclass in a `_abc_registry` frozenset attribute on the
                // class; `isinstance`/`issubclass` consult it (see
                // `builtin_isinstance`/`builtin_issubclass`). Real trigger:
                // `numbers.Number.register(Decimal)` — needed by real
                // CPython's own (vendored) `_pydecimal.py`.
                if name == "register" && !dict.contains_key_str("register") {
                    return Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "register".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "register() takes exactly one argument",
                                ));
                            }
                            let cls = &args[0];
                            let subclass = args[1].clone();
                            // Read the registry from `cls`'s OWN dict only
                            // — NOT via `get_attribute` (which walks the
                            // MRO). `Real.register(float)` must not see
                            // (and then re-save as ITS OWN registry,
                            // permanently merging the two) whatever
                            // `Complex.register(complex)` already stored,
                            // just because `Real` is a subclass of
                            // `Complex` and doesn't have its own registry
                            // entry yet. Confirmed via `numbers.py`'s own
                            // `Complex.register(complex)`/`Real.register
                            // (float)`/`Integral.register(int)`: without
                            // this, `Integral._abc_registry` ended up
                            // accumulating `{complex, float, int}` (all
                            // three merged in), making `issubclass(complex,
                            // Integral)` wrongly `True`.
                            let existing: Vec<PyObjectRef> =
                                if let PyObject::Type { dict, .. } = &*cls.borrow() {
                                    dict.get_str("_abc_registry")
                                        .and_then(|r| {
                                            if let PyObject::FrozenSet(items) = &*r.borrow() {
                                                Some(items.to_vec())
                                            } else {
                                                None
                                            }
                                        })
                                        .unwrap_or_default()
                                } else {
                                    Vec::new()
                                };
                            if !existing.iter().any(|r| r.is(&subclass)) {
                                let mut set = PySet::new();
                                for item in &existing {
                                    set.add(item.clone())?;
                                }
                                set.add(subclass.clone())?;
                                cls.borrow_mut().set_attribute(
                                    "_abc_registry",
                                    PyObjectRef::imm(PyObject::FrozenSet(set)),
                                )?;
                            }
                            Ok(subclass)
                        },
                        self_obj: py_none(),
                    }));
                }
                if name == "__subclasses__" && !dict.contains_key_str("__subclasses__") {
                    // NOTE: self_obj here is a placeholder — LOAD_ATTR's fast
                    // path always rebinds it to the actual accessed object
                    // (`Foo`, for `Foo.__subclasses__`) before calling, so the
                    // real class must be read back out of args[0] at call time
                    // (matching the `mro` method right below).
                    return Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__subclasses__".to_string(),
                        func: |args| Ok(py_list(direct_subclasses_of(&args[0]))),
                        self_obj: py_none(),
                    }));
                }
                if name == "mro" && !dict.contains_key_str("mro") {
                    // NOTE: self_obj here is a placeholder — LOAD_ATTR's fast
                    // path always rebinds it to the actual accessed object
                    // (`Foo`, for `Foo.mro`) before calling, so the real mro
                    // must be read back out of args[0] at call time.
                    return Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "mro".to_string(),
                        func: |args| {
                            if let PyObject::Type { mro, .. } = &*args[0].borrow() {
                                Ok(py_list(mro.clone()))
                            } else {
                                Err(PyError::type_error("mro() requires a type object"))
                            }
                        },
                        self_obj: py_none(),
                    }));
                }
                if std::env::var("RPY_DEBUG_TYPEATTR").is_ok() && name == "strip" {
                    eprintln!(
                        "TYPEATTR type={} name={} dict_has={}",
                        type_name,
                        name,
                        dict.contains_key_str(&name)
                    );
                }
                // Check own dict first
                if let Some(val) = dict.get_str(&name).cloned() {
                    // Unwrap staticmethod descriptor so type access returns the function directly
                    let b = val.borrow();
                    if let PyObject::StaticMethod { func } = &*b {
                        return Ok(func.clone());
                    }
                    drop(b);
                    return Ok(val);
                }
                // Check MRO (skip self)
                for base in mro.iter().skip(1) {
                    if let PyObject::Type {
                        dict: base_dict, ..
                    } = &*base.borrow()
                    {
                        if let Some(val) = base_dict.get_str(&name) {
                            // Unwrap staticmethod descriptor from MRO bases
                            let b = val.borrow();
                            if let PyObject::StaticMethod { func } = &*b {
                                return Ok(func.clone());
                            }
                            drop(b);
                            return Ok(val.clone());
                        }
                    }
                }
                // Fallback: for dict-derived types, provide common dict methods
                if name == "__iter__"
                    || name == "items"
                    || name == "keys"
                    || name == "values"
                    || name == "get"
                {
                    static DICT_METHODS: std::sync::OnceLock<
                        std::collections::HashMap<String, BuiltinFunc>,
                    > = std::sync::OnceLock::new();
                    let methods = DICT_METHODS.get_or_init(|| {
                        let mut m = std::collections::HashMap::new();
                        m.insert("__iter__".to_string(), dict_method_iter as BuiltinFunc);
                        m.insert("items".to_string(), dict_method_items as BuiltinFunc);
                        m.insert("keys".to_string(), dict_method_keys as BuiltinFunc);
                        m.insert("values".to_string(), dict_method_values as BuiltinFunc);
                        m.insert("get".to_string(), dict_method_get as BuiltinFunc);
                        m
                    });
                    if let Some(func) = methods.get(name) {
                        let func = *func;
                        // A plain `BuiltinFunction`, NOT `BuiltinMethod` — this
                        // is reached via `dict.keys` (attribute access on the
                        // TYPE itself, for the unbound-call idiom `dict.keys
                        // (self)` a dict subclass uses to invoke the parent's
                        // real implementation) rather than `some_dict.keys()`
                        // (bound instance access, handled elsewhere). A
                        // `BuiltinMethod`'s calling convention prepends its
                        // OWN `self_obj` ahead of whatever args the caller
                        // passes — with a `py_none()` placeholder here, that
                        // shifted every real argument by one (`dict.keys(d)`
                        // called `dict_method_keys(&[None, d])`, so `args[0]`
                        // was never `d` at all) — confirmed via direct repro
                        // (`dict.keys({'a': 1})` unconditionally failed).
                        // `BuiltinFunction`'s plain pass-through convention is
                        // what an unbound-style call actually needs.
                        return Ok(PyObjectRef::new(PyObject::BuiltinFunction {
                            name: name.to_string(),
                            func,
                        }));
                    }
                }
                Err(PyError::attribute_error(format!(
                    "type has no attribute '{}'",
                    name
                )))
            }
            PyObject::Instance { dict, typ } => {
                if name == "__dict__" {
                    // Return a copy of the instance's HashMap as a PyDict (no
                    // live view from here) — NATIVE_BACKING_KEY is internal
                    // bookkeeping (see native_backing_of) and must not leak
                    // into user-visible introspection.
                    let mut pd = PyDict::new();
                    for (k, v) in dict.iter() {
                        if k == NATIVE_BACKING_KEY {
                            continue;
                        }
                        let _ = pd.set(py_str(k), v.clone());
                    }
                    return Ok(PyObjectRef::new(PyObject::Dict(Box::new(pd))));
                }
                if name == "__weakref__" {
                    // __weakref__ slot exists but returns None by default
                    // A full implementation would return a WeakRef object if one exists
                    return Ok(py_none());
                }
                // If __slots__ is defined, verify the attribute is allowed
                if let Some(slots) = get_instance_slots(typ) {
                    if !slots.iter().any(|s| s == name) {
                        // Check if it's a class-level attribute (method, etc.) — those are always allowed
                        let typ_ref = typ.borrow();
                        let is_in_type = if let PyObject::Type {
                            dict: type_dict,
                            mro,
                            ..
                        } = &*typ_ref
                        {
                            type_dict.contains_key_str(&name)
                                || mro.iter().skip(1).any(|base| {
                                    if let PyObject::Type {
                                        dict: base_dict, ..
                                    } = &*base.borrow()
                                    {
                                        base_dict.contains_key_str(&name)
                                    } else {
                                        false
                                    }
                                })
                        } else {
                            false
                        };
                        if !is_in_type {
                            let type_name = get_type_name_for_instance(typ);
                            return Err(PyError::attribute_error(format!(
                                "'{}' object has no attribute '{}'",
                                type_name, name
                            )));
                        }
                    }
                }
                dict.get_str(&name).cloned().or_else(|| {
                    let typ_ref = typ.borrow();
                    if let PyObject::Type { dict: type_dict, mro, .. } = &*typ_ref {
                        type_dict.get_str(&name).cloned().or_else(|| {
                            for base in mro.iter().skip(1) {
                                if let PyObject::Type { dict: base_dict, .. } = &*base.borrow() {
                                    if let Some(val) = base_dict.get_str(&name) {
                                        return Some(val.clone());
                                    }
                                }
                            }
                            // Not overridden anywhere in the mro: for a class
                            // that transparently subclasses list/dict/str
                            // (`class Foo(list): ...`), delegate to the real
                            // native value's own attribute resolution. Its
                            // get_attribute returns a placeholder self_obj
                            // (the real binding normally happens wherever
                            // LOAD_ATTR was invoked, rebinding to whatever it
                            // was accessed on) — here that must be rebound to
                            // the native backing itself, not this instance,
                            // or mutations would target the placeholder. This
                            // must run BEFORE the generic dict-like fallback
                            // below, which would otherwise misinterpret the
                            // native backing's own dict entry as plain
                            // instance-attribute data.
                            if let Some(native) = dict.get(NATIVE_BACKING_KEY) {
                                // A deque subclass's `__copy__`/`copy()` must
                                // return a NEW instance of the SAME subclass
                                // (real CPython: `D('abc').__copy__()` is a
                                // `D`), not a raw deque — the generic native
                                // delegation below would rebind `self_obj` to
                                // the backing deque and build a plain deque.
                                if matches!(&*native.borrow(), PyObject::Deque { .. }) && (name == "__copy__" || name == "copy") {
                                    let typ_clone = typ.clone();
                                    let new_native = {
                                        let b = native.borrow();
                                        if let PyObject::Deque { data, maxlen } = &*b {
                                            py_deque(data.clone(), *maxlen)
                                        } else { unreachable!() }
                                    };
                                    return Some(PyObjectRef::new(PyObject::Closure(Rc::new(move |_args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                                        let mut new_dict = AttrMap::new();
                                        new_dict.insert(NATIVE_BACKING_KEY.to_string(), new_native.clone());
                                        Ok(PyObjectRef::new(PyObject::Instance { typ: typ_clone.clone(), dict: new_dict }))
                                    }))));
                                }
                                if name == "__buffer__" || name == "__release_buffer__" {
                                    // Skip delegation for buffer protocol - let VM handle it
                                } else if let Ok(val) = native.borrow().get_attribute(name) {
                                    let rebound = if let PyObject::BuiltinMethod { name: n, func, .. } = &*val.borrow() {
                                        PyObjectRef::imm(PyObject::BuiltinMethod { name: n.clone(), func: *func, self_obj: native.clone() })
                                    } else {
                                        val.clone()
                                    };
                                    return Some(rebound);
                                }
                            }
                            // Fallback: provide common dict methods for dict-like instances
                            if name == "__iter__" || name == "items" || name == "keys" || name == "values" {
                                let dict_snapshot: Vec<(String, PyObjectRef)> = dict.iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
                                let result = instance_builtin_dict_method(name, dict_snapshot);
                                return result;
                            }
                            // PEP 3134 traceback/chaining protocol methods and
                            // attributes for a USER-DEFINED exception class
                            // (`class MyError(Exception): ...`) that doesn't
                            // override them itself — the native
                            // `PyObject::Exception` representation already
                            // has these (see its own `get_attribute_impl`
                            // arm), but a custom subclass is a plain
                            // `PyObject::Instance` and fell straight through
                            // to `AttributeError` for all of them. Real
                            // trigger: `unittest`'s own `assertRaises`
                            // (`_AssertRaisesBaseContext.__exit__`) calling
                            // `exc_value.with_traceback(None)` on WHATEVER
                            // exception it caught — this raised
                            // `AttributeError` for literally any
                            // user-defined exception class, only working by
                            // accident for the handful of natively-
                            // represented ones.
                            if matches!(name, "with_traceback" | "add_note" | "__traceback__" | "__context__" | "__cause__" | "__suppress_context__" | "__notes__")
                                && find_exception_base_name(typ).is_some() {
                                return Some(match name {
                                    "with_traceback" => PyObjectRef::imm(PyObject::BuiltinMethod {
                                        name: "with_traceback".to_string(),
                                        func: |args| {
                                            if args.len() < 2 { return Err(PyError::type_error("with_traceback() takes exactly one argument")); }
                                            Ok(args[0].clone())
                                        },
                                        self_obj: PyObjectRef::new(PyObject::None),
                                    }),
                                    "add_note" => PyObjectRef::imm(PyObject::BuiltinMethod {
                                        name: "add_note".to_string(),
                                        func: |_args| Ok(py_none()),
                                        self_obj: PyObjectRef::new(PyObject::None),
                                    }),
                                    // `__cause__` was missing from this list entirely
                                    // (only `__context__`/`__traceback__` had a
                                    // fallback) — any user-defined exception class
                                    // reading its own `.__cause__` before ever
                                    // setting it (e.g. `raise X from Y` wasn't used)
                                    // raised `AttributeError` instead of `None`. Real
                                    // trigger: CPython's own doctest/exception-group
                                    // test files reading `.__cause__` on a plain
                                    // user-defined exception instance.
                                    "__context__" | "__traceback__" | "__cause__" => py_none(),
                                    "__suppress_context__" => py_bool(false),
                                    "__notes__" => py_list(vec![]),
                                    _ => unreachable!(),
                                });
                            }
                            None
                        })
                    } else {
                        None
                    }
                }).ok_or_else(|| PyError::attribute_error(format!("'{}' object has no attribute '{}'", get_type_name_for_instance(typ), name)))
            }
            PyObject::Property(ref d) => {
                let getter = &d.getter;
                let setter = &d.setter;
                let deleter = &d.deleter;
                let doc = &d.doc;
                match name {
                    "fget" => getter.clone().ok_or_else(|| {
                        PyError::attribute_error("property has no getter".to_string())
                    }),
                    "fset" => setter.clone().ok_or_else(|| {
                        PyError::attribute_error("property has no setter".to_string())
                    }),
                    "fdel" => deleter.clone().ok_or_else(|| {
                        PyError::attribute_error("property has no deleter".to_string())
                    }),
                    "doc" | "__doc__" => Ok(doc.clone().map_or_else(py_none, |d| py_str(&d))),
                    "__get__" => {
                        if let Some(_) = getter {
                            Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                                name: "__get__".to_string(),
                                func: |args| {
                                    if args.len() < 4 {
                                        return Err(PyError::type_error(
                                            "__get__() takes 2 positional arguments",
                                        ));
                                    }
                                    // args: [self_obj, descriptor, instance, owner]
                                    let g = args[1].borrow();
                                    if let PyObject::Property(ref data) = &*g {
                                        if let Some(ref getter_fn) = data.getter {
                                            call_bound_method(
                                                getter_fn.clone(),
                                                args[2].clone(),
                                                vec![],
                                            )
                                        } else {
                                            Err(PyError::runtime_error("property has no getter"))
                                        }
                                    } else {
                                        Err(PyError::runtime_error("property has no getter"))
                                    }
                                },
                                self_obj: PyObjectRef::new(PyObject::None),
                            }))
                        } else {
                            Err(PyError::attribute_error(
                                "property has no getter".to_string(),
                            ))
                        }
                    }
                    "__set__" => {
                        if let Some(_) = setter {
                            Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                                name: "__set__".to_string(),
                                func: |args| {
                                    if args.len() < 4 {
                                        return Err(PyError::type_error(
                                            "__set__() takes 2 positional arguments",
                                        ));
                                    }
                                    // args: [self_obj, descriptor, instance, value]
                                    let s = args[1].borrow();
                                    if let PyObject::Property(ref data) = &*s {
                                        if let Some(ref setter_fn) = data.setter {
                                            call_bound_method(
                                                setter_fn.clone(),
                                                args[2].clone(),
                                                vec![args[3].clone()],
                                            )
                                        } else {
                                            Err(PyError::runtime_error("property has no setter"))
                                        }
                                    } else {
                                        Err(PyError::runtime_error("property has no setter"))
                                    }
                                },
                                self_obj: PyObjectRef::new(PyObject::None),
                            }))
                        } else {
                            Err(PyError::attribute_error(
                                "property has no setter".to_string(),
                            ))
                        }
                    }
                    "setter" | "deleter" | "getter" => {
                        let is_setter = name == "setter";
                        let prop_obj = PyObjectRef::new(match self {
                            PyObject::Property(ref d) => {
                                PyObject::Property(Box::new(PropertyData {
                                    getter: d.getter.clone(),
                                    setter: d.setter.clone(),
                                    deleter: d.deleter.clone(),
                                    doc: d.doc.clone(),
                                }))
                            }
                            _ => unreachable!(),
                        });
                        Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                            name: name.to_string(),
                            func: if is_setter {
                                builtin_property_setter_fn
                            } else {
                                builtin_property_deleter_fn
                            },
                            self_obj: prop_obj,
                        }))
                    }
                    // `property.__isabstractmethod__` — real Python's ABC
                    // machinery (`abc.update_abstractmethods`, `ABCMeta`
                    // itself) checks this to recognize `@property
                    // @abstractmethod def foo(self): ...`-style abstract
                    // properties; missing entirely raised `AttributeError`
                    // for even the most basic ABC property test (real
                    // trigger: CPython's own `test_abc.py`'s
                    // `test_abstractproperty_basics`). True iff ANY of
                    // getter/setter/deleter is itself marked abstract,
                    // matching real CPython's own `property.__isabstractmethod__`.
                    "__isabstractmethod__" => {
                        fn is_abstract(f: &Option<PyObjectRef>) -> bool {
                            f.as_ref()
                                .and_then(|func| {
                                    func.borrow().get_attribute("__isabstractmethod__").ok()
                                })
                                .map(|v| v.truthy())
                                .unwrap_or(false)
                        }
                        Ok(py_bool(
                            is_abstract(getter) || is_abstract(setter) || is_abstract(deleter),
                        ))
                    }
                    _ => Err(PyError::attribute_error(format!(
                        "'property' object has no attribute '{}'",
                        name
                    ))),
                }
            }
            // `classmethod`/`staticmethod` had NO dedicated attribute-access
            // arm at all — any attribute access (`.__func__`,
            // `.__isabstractmethod__`) fell through to a generic
            // "not callable"/catch-all failure. `__isabstractmethod__` is
            // the one real trigger found (CPython's own `test_abc.py`'s
            // `test_abstractclassmethod_basics`/`test_abstractstaticmethod_basics`);
            // `__func__` (the real CPython attribute exposing the wrapped
            // function) added alongside it since it's the same shape of gap
            // and trivial to expose from the same field.
            PyObject::StaticMethod { func } => {
                match name {
                    "__func__" => Ok(func.clone()),
                    "__wrapped__" => Ok(func.clone()),
                    "__isabstractmethod__" => Ok(py_bool(
                        func.borrow()
                            .get_attribute("__isabstractmethod__")
                            .map(|v| v.truthy())
                            .unwrap_or(false),
                    )),
                    // `__name__`/`__module__`/`__qualname__`/`__doc__`/
                    // `__annotations__`/`__dict__` all delegate to the
                    // wrapped callable (test_decorators.py's
                    // check_wrapper_attrs asserts them on the descriptor).
                    _ => func.borrow().get_attribute(name).map_err(|_| {
                        PyError::attribute_error(format!(
                            "'staticmethod' object has no attribute '{}'",
                            name
                        ))
                    }),
                }
            }
            PyObject::ClassMethod { func } => match name {
                "__func__" => Ok(func.clone()),
                "__wrapped__" => Ok(func.clone()),
                "__isabstractmethod__" => Ok(py_bool(
                    func.borrow()
                        .get_attribute("__isabstractmethod__")
                        .map(|v| v.truthy())
                        .unwrap_or(false),
                )),
                _ => func.borrow().get_attribute(name).map_err(|_| {
                    PyError::attribute_error(format!(
                        "'classmethod' object has no attribute '{}'",
                        name
                    ))
                }),
            },
            PyObject::Exception {
                typ,
                args,
                cause,
                suppress_context,
                context,
                traceback,
                extra,
            } => {
                match name {
                    "__name__" => Ok(py_str(typ)),
                    "args" => Ok(py_tuple(args.clone())),
                    // `StopIteration.value` (and StopAsyncIteration) — the
                    // value a generator/coroutine returned (real code: a
                    // driver does `coro.send(None)` and reads `e.value`).
                    // Real CPython: StopIteration(value).value == value.
                    "value" if typ == "StopIteration" || typ == "StopAsyncIteration" => {
                        if args.len() == 1 {
                            Ok(args[0].clone())
                        } else if args.is_empty() {
                            Ok(py_none())
                        } else {
                            Ok(py_tuple(args.clone()))
                        }
                    }
                    // `lineno`/`offset` — a real SyntaxError carries its
                    // source position (test.support's check_syntax_error
                    // asserts both are not None). The parser's error
                    // messages embed "L<line>:<col>:" as a prefix; parse it
                    // out lazily. Defaults to None for non-syntax errors.
                    "lineno" | "offset" => {
                        let want_lineno = name == "lineno";
                        // A ctor-set SyntaxError location tuple wins over the
                        // lazy "L<line>:<col>:" parsing below.
                        if typ == "SyntaxError" {
                            if let Some(extra) = extra {
                                if let Some(v) = extra.get(name) {
                                    return Ok(v.clone());
                                }
                            }
                        }
                        let parsed = args.first().and_then(|a| {
                            let s = a.str();
                            if let Some(rest) = s.strip_prefix('L') {
                                let (ln, rest) = rest.split_once(':')?;
                                let (col, _rest) = rest.split_once(':')?;
                                let line = ln.parse::<i64>().ok()?;
                                let offset = col.parse::<i64>().ok()?;
                                Some((line, offset))
                            } else {
                                None
                            }
                        });
                        match parsed {
                            Some((line, offset)) => {
                                Ok(py_int(if want_lineno { line } else { offset }))
                            }
                            None => Ok(py_none()),
                        }
                    }
                    // `encoding`/`object`/`start`/`end`/`reason` — the
                    // UnicodeError family's five positional args
                    // (UnicodeEncodeError('utf-8', obj, start, end, reason));
                    // codec error-handler functions (backslashreplace_errors
                    // etc.) read these.
                    "encoding" | "object" | "start" | "end" | "reason"
                        if typ == "UnicodeError"
                            || typ == "UnicodeEncodeError"
                            || typ == "UnicodeDecodeError"
                            || typ == "UnicodeTranslateError" =>
                    {
                        let idx = match name {
                            "encoding" => 0,
                            "object" => 1,
                            "start" => 2,
                            "end" => 3,
                            _ => 4,
                        };
                        match args.get(idx) {
                            Some(v) => Ok(v.clone()),
                            None => Ok(py_none()),
                        }
                    }
                    // `__str__`/`__repr__` — real exceptions always expose
                    // both (test_baseexception's verify_instance_interface
                    // asserts `args`/`__str__`/`__repr__` on EVERY builtin
                    // exception instance). CPython: str(exc) joins str(args)
                    // (empty args -> empty string); repr is `TypeName(args)`.
                    "__str__" => {
                        let parts: Vec<String> = args.iter().map(|a| a.str()).collect();
                        Ok(py_str(&parts.join(", ")))
                    }
                    "__repr__" => {
                        let parts: Vec<String> = args.iter().map(|a| a.repr()).collect();
                        Ok(py_str(&format!("{}({})", typ, parts.join(", "))))
                    }
                    "__cause__" => match cause {
                        Some(cause_exc) => Ok(cause_exc.clone()),
                        None => Ok(py_none()),
                    },
                    "__context__" => match context {
                        Some(ctx_exc) => Ok(ctx_exc.clone()),
                        None => Ok(py_none()),
                    },
                    // PEP 3134 implicit exception chaining/traceback
                    // attributes every real exception instance carries
                    // (defaulting to `None`/`False`) — this interpreter
                    // doesn't implement implicit `__context__` capture (an
                    // exception raised while another is being handled)
                    // or a real traceback OBJECT, but code that merely
                    // checks these are present/None (real trigger:
                    // `unittest`'s own `TestResult._clean_tracebacks`,
                    // `for c in (value.__cause__, value.__context__): if c
                    // is not None: ...`) previously raised AttributeError
                    // just from the attribute not existing at all.
                    "__traceback__" => match traceback {
                        Some(tb) => Ok(tb.clone()),
                        None => Ok(py_none()),
                    },
                    "__suppress_context__" => Ok(py_bool(*suppress_context)),
                    "__notes__" => {
                        if let Some(extra) = extra {
                            if let Some(v) = extra.get("__notes__") {
                                return Ok(v.clone());
                            }
                        }
                        Ok(py_list(vec![]))
                    }
                    // Per-instance attributes (BaseException.__dict__): the
                    // constructor's keyword args (`AttributeError('x',
                    // name=..., obj=...)`) and anything assigned by user
                    // code. `__dict__` returns a copy; name/obj are the
                    // AttributeError-specific ones CPython's test_exceptions
                    // asserts.
                    "__dict__" => {
                        let mut d = crate::object::PyDict::new();
                        if let Some(extra) = extra {
                            for (k, v) in extra.iter() {
                                let _ = d.set(py_str(k), v.clone());
                            }
                        }
                        Ok(PyObjectRef::new(PyObject::Dict(Box::new(d))))
                    }
                    // `BaseException.__setstate__(state)` — inherited by
                    // every exception, used by pickle to restore extra
                    // instance attributes on unpickling. Real semantics:
                    // `None` is a no-op, a `dict` merges into `__dict__`,
                    // anything else raises `TypeError` (found via CPython's
                    // own `test_exceptions.py::test_invalid_setstate`, which
                    // checks exactly this error case). Exceptions here have
                    // no generic attribute-dict storage the way `Instance`
                    // does, so a valid dict argument is accepted (matching
                    // real behavior/not raising) but not actually persisted
                    // — a narrower, deliberate limitation, not the gap this
                    // fix targets.
                    "__setstate__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__setstate__".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "__setstate__() takes exactly one argument",
                                ));
                            }
                            match &*args[1].borrow() {
                                PyObject::None => Ok(py_none()),
                                PyObject::Dict(_) => {
                                    // Merge the state dict into the exception's
                                    // per-instance attrs (BaseException
                                    // `__dict__`), with the special `args` key
                                    // REPLACING the exception's args tuple —
                                    // pickle round-trips and
                                    // `e.__setstate__({'a': 1, 'args': (...)})`
                                    // work (test_exceptions::test_setstate).
                                    let mut m = std::collections::HashMap::new();
                                    if let PyObject::Dict(d) = &*args[1].borrow() {
                                        for (k, v) in d.iter() {
                                            let key = match &*k.borrow() {
                                                PyObject::Str(s) => s.to_string(),
                                                _ => continue,
                                            };
                                            m.insert(key, v.clone());
                                        }
                                    }
                                    let new_args = m.remove("args");
                                    if let PyObject::Exception { args, extra, .. } =
                                        &mut *args[0].borrow_mut()
                                    {
                                        if let Some(na) = new_args {
                                            let is_tuple =
                                                matches!(&*na.borrow(), PyObject::Tuple(_));
                                            let cloned = if is_tuple {
                                                match &*na.borrow() {
                                                    PyObject::Tuple(t) => t.clone(),
                                                    _ => unreachable!(),
                                                }
                                            } else {
                                                vec![na.clone()]
                                            };
                                            *args = cloned;
                                        }
                                        if !m.is_empty() {
                                            let store = extra.get_or_insert_with(|| {
                                                std::collections::HashMap::new()
                                            });
                                            for (k, v) in m {
                                                store.insert(k, v);
                                            }
                                        }
                                    }
                                    Ok(py_none())
                                }
                                _ => Err(PyError::type_error("state is not a dictionary")),
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "add_note" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "add_note".to_string(),
                        func: |_args| Ok(py_none()),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "with_traceback" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "with_traceback".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "with_traceback() takes exactly one argument",
                                ));
                            }
                            // Store the traceback so `raise X().with_traceback(tb)`
                            // yields `X.__traceback__` chaining tb (the RAISE
                            // unwind prepends the current frame's own node).
                            args[0]
                                .borrow_mut()
                                .set_attribute("__traceback__", args[1].clone())?;
                            Ok(args[0].clone())
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    // `e.__init__(*args)` — re-initializes the exception:
                    // replaces `.args` and resets per-instance attrs
                    // (test_reset_attributes: `exc.__init__()` clears
                    // msg/name/path). Returns None like object.__init__.
                    "__init__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__init__".to_string(),
                        func: |args| {
                            if let PyObject::Exception { args: a, extra, .. } =
                                &mut *args[0].borrow_mut()
                            {
                                *a = args.get(1..).unwrap_or(&[]).to_vec();
                                *extra = None;
                            }
                            Ok(py_none())
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    // `SyntaxError`'s extra attributes (`filename`/`lineno`/
                    // `offset`/`text`/`end_lineno`/`end_offset`) — this
                    // interpreter's own `syntax_error()` constructor
                    // (`errors.rs`) doesn't thread real source-location data
                    // through from the parser/compiler at all, so these
                    // can't carry genuine values yet — but real Python code
                    // that merely reads them (real trigger: CPython's own
                    // `test_exceptions.py`) previously got `AttributeError`
                    // instead of `None`, which is what real CPython itself
                    // returns for a `SyntaxError` constructed without the
                    // extra positional-args tuple. Gated to `SyntaxError`
                    // specifically — a plain `Exception`/`ValueError`/etc.
                    // genuinely has no such attributes in real Python either.
                    // `SyntaxError`'s location attributes come from the
                    // ctor's 6-tuple (`msg`, `filename`, `lineno`, `offset`,
                    // `text`, `end_lineno`, `end_offset`); reading them
                    // falls back to None when never set. `msg` additionally
                    // defaults to the first positional arg (`SyntaxError
                    // ('msgStr')` -> `.msg == 'msgStr'`).
                    "msg"
                    | "filename"
                    | "lineno"
                    | "offset"
                    | "text"
                    | "end_lineno"
                    | "end_offset"
                    | "print_file_and_line"
                        if typ == "SyntaxError" =>
                    {
                        if let Some(extra) = extra {
                            if let Some(v) = extra.get(name) {
                                return Ok(v.clone());
                            }
                        }
                        if name == "msg" {
                            if let Some(first) = args.first() {
                                return Ok(first.clone());
                            }
                        }
                        Ok(py_none())
                    }
                    // `AttributeError.name`/`.obj` default to None when not
                    // set by the constructor or getattr machinery.
                    "name" | "obj" if typ == "AttributeError" => {
                        if let Some(extra) = extra {
                            if let Some(v) = extra.get(name) {
                                return Ok(v.clone());
                            }
                        }
                        Ok(py_none())
                    }
                    // `ImportError.name`/`.path` (ctor kwargs, default None)
                    // and `.msg` (alias for args[0]).
                    "name" | "path" if typ == "ImportError" => {
                        if let Some(extra) = extra {
                            if let Some(v) = extra.get(name) {
                                return Ok(v.clone());
                            }
                        }
                        Ok(py_none())
                    }
                    "msg" if typ == "ImportError" => {
                        if let Some(first) = args.first() {
                            return Ok(first.clone());
                        }
                        Ok(py_none())
                    }
                    // `OSError.errno`/`.strerror`/`.filename`/`.filename2`
                    // (derived from the ctor's positional args).
                    "errno" | "strerror" | "filename" | "filename2"
                        if typ == "OSError" || typ == "EnvironmentError" =>
                    {
                        if let Some(extra) = extra {
                            if let Some(v) = extra.get(name) {
                                return Ok(v.clone());
                            }
                        }
                        Ok(py_none())
                    }
                    // `SystemExit.code` — args[0] when present, else None.
                    "code" if typ == "SystemExit" => {
                        Ok(args.first().cloned().unwrap_or_else(py_none))
                    }
                    // `NameError.name` — the undefined name (set by the VM's
                    // LOAD_NAME path), default None.
                    "name" if typ == "NameError" || typ == "UnboundLocalError" => {
                        if let Some(extra) = extra {
                            if let Some(v) = extra.get(name) {
                                return Ok(v.clone());
                            }
                        }
                        Ok(py_none())
                    }
                    _ => {
                        // Per-instance extras (BaseException.__dict__) —
                        // e.g. `AttributeError('x', name='carry').name`.
                        if let Some(extra) = extra {
                            if let Some(v) = extra.get(name) {
                                return Ok(v.clone());
                            }
                        }
                        Err(PyError::attribute_error(format!(
                            "'{}' object has no attribute '{}'",
                            typ, name
                        )))
                    }
                }
            }
            // `ExceptionGroup`/`BaseExceptionGroup` (PEP 654) had NO
            // attribute access implemented at all — not even the two core
            // PEP 654 fields (`.message`, `.exceptions`), let alone the
            // same PEP 3134 chaining/traceback attributes `Exception`
            // itself already supports just above. Real trigger: CPython's
            // own `test_exception_group.py` — even the most basic
            // `ExceptionGroup("msg", [...]).message` raised `AttributeError`.
            PyObject::ExceptionGroup { .. } => return exception_group::get(self, name),
            PyObject::List(_v) => return list::get(self, name),
            PyObject::Deque { data, maxlen } => return deque::get(self, name),
            PyObject::Tuple(_v) => return tuple::get(self, name),
            PyObject::Bytes(_v) => return bytes1::get(self, name),
            PyObject::ByteArray(_b) => {
                match name {
                    "__mul__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__mul__".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error("__mul__() missing argument"));
                            }
                            dunder_repeat(
                                &args[args.len().saturating_sub(2)],
                                &args[args.len().saturating_sub(1)],
                            )
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__rmul__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__rmul__".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error("__rmul__() missing argument"));
                            }
                            dunder_repeat(
                                &args[args.len().saturating_sub(2)],
                                &args[args.len().saturating_sub(1)],
                            )
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__contains__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__contains__".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "__contains__() takes exactly one argument",
                                ));
                            }
                            py_contains(&args[0], &args[1])
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__mod__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__mod__".to_string(),
                        func: |args| {
                            if args.len() < 3 {
                                return Err(PyError::type_error("__mod__() too few args"));
                            }
                            let fmt = match &*args[1].borrow() {
                                PyObject::ByteArray(b) => b.clone(),
                                _ => {
                                    return Err(PyError::runtime_error("__mod__ on non-bytearray"))
                                }
                            };
                            let result = bytes_interpolate(&fmt, &args[2]).map_err(|e| {
                                if e.contains("too big") || e.contains("[overflow]") {
                                    PyError::overflow_error(
                                        e.trim_end_matches(" [overflow]").to_string(),
                                    )
                                } else if e.contains("unsupported format character")
                                    || e == "incomplete format"
                                {
                                    PyError::value_error(e)
                                } else {
                                    PyError::type_error(e)
                                }
                            })?;
                            Ok(PyObjectRef::new(PyObject::ByteArray(result)))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "append" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "append".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "append() takes exactly one argument",
                                ));
                            }
                            let val = args[1].borrow();
                            if let PyObject::Int(i) = &*val {
                                let n = i.to_i64().ok_or_else(|| {
                                    PyError::value_error("byte value out of range")
                                })?;
                                if n < 0 || n > 255 {
                                    return Err(PyError::value_error(
                                        "byte must be in range(0, 256)",
                                    ));
                                }
                                if crate::object::is_bytearray_exported(&args[0]) { eprintln!("extend BufferError"); return Err(PyError::buffer_error("Existing exports of data: object cannot be re-sized")); } eprintln!("extend not exported, proceeding");
                                if let PyObject::ByteArray(bytes) = &mut *args[0].borrow_mut() {
                                    bytes.push(n as u8);
                                    Ok(py_none())
                                } else {
                                    Err(PyError::runtime_error("append on non-bytearray"))
                                }
                            } else {
                                Err(PyError::type_error("argument must be an integer"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "extend" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "extend".to_string(),
                        func: |args| {
                                                        if crate::object::is_bytearray_exported(&args[0]) {
                                return Err(PyError::buffer_error("Existing exports of data: object cannot be re-sized"));
                            }
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "extend() takes exactly one argument",
                                ));
                            }
                            let it = builtin_iter(&[args[1].clone()])?;
                            loop {
                                match builtin_next(&[it.clone()]) {
                                    Ok(v) => {
                                        let vv = v.borrow();
                                        if let PyObject::Int(i) = &*vv {
                                            let n = i.to_i64().ok_or_else(|| {
                                                PyError::value_error("byte value out of range")
                                            })?;
                                            if n < 0 || n > 255 {
                                                return Err(PyError::value_error(
                                                    "byte must be in range(0, 256)",
                                                ));
                                            }
                                            if let PyObject::ByteArray(bytes) =
                                                &mut *args[0].borrow_mut()
                                            {
                                                bytes.push(n as u8);
                                            } else {
                                                return Err(PyError::runtime_error(
                                                    "extend on non-bytearray",
                                                ));
                                            }
                                        } else {
                                            return Err(PyError::type_error(
                                                "argument must be iterable of integers",
                                            ));
                                        }
                                    }
                                    Err(PyError::StopIteration) => return Ok(py_none()),
                                    Err(e) => return Err(e),
                                }
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "insert" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "insert".to_string(),
                        func: |args| {
                            if args.len() < 3 {
                                return Err(PyError::type_error(
                                    "insert() takes exactly 2 arguments",
                                ));
                            }
                            let idx = args[1].as_i64().unwrap_or(0) as usize;
                            let val = args[2].borrow();
                            if let PyObject::Int(i) = &*val {
                                let n = i.to_i64().ok_or_else(|| {
                                    PyError::value_error("byte value out of range")
                                })?;
                                if n < 0 || n > 255 {
                                    return Err(PyError::value_error(
                                        "byte must be in range(0, 256)",
                                    ));
                                }
                                if crate::object::is_bytearray_exported(&args[0]) { eprintln!("extend BufferError"); return Err(PyError::buffer_error("Existing exports of data: object cannot be re-sized")); } eprintln!("extend not exported, proceeding");
                                if let PyObject::ByteArray(bytes) = &mut *args[0].borrow_mut() {
                                    let idx = idx.min(bytes.len());
                                    bytes.insert(idx, n as u8);
                                    Ok(py_none())
                                } else {
                                    Err(PyError::runtime_error("insert on non-bytearray"))
                                }
                            } else {
                                Err(PyError::type_error("argument must be an integer"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "remove" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "remove".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "remove() takes exactly one argument",
                                ));
                            }
                            let val = args[1].borrow();
                            if let PyObject::Int(i) = &*val {
                                let n = i.to_i64().ok_or_else(|| {
                                    PyError::value_error("byte value out of range")
                                })? as u8;
                                if crate::object::is_bytearray_exported(&args[0]) { eprintln!("extend BufferError"); return Err(PyError::buffer_error("Existing exports of data: object cannot be re-sized")); } eprintln!("extend not exported, proceeding");
                                if let PyObject::ByteArray(bytes) = &mut *args[0].borrow_mut() {
                                    let pos =
                                        bytes.iter().position(|&x| x == n).ok_or_else(|| {
                                            PyError::value_error(format!(
                                                "value {} not found in bytearray",
                                                n
                                            ))
                                        })?;
                                    bytes.remove(pos);
                                    Ok(py_none())
                                } else {
                                    Err(PyError::runtime_error("remove on non-bytearray"))
                                }
                            } else {
                                Err(PyError::type_error("argument must be an integer"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "pop" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "pop".to_string(),
                        func: |args| {
                            if crate::object::is_bytearray_exported(&args[0]) { eprintln!("extend BufferError"); return Err(PyError::buffer_error("Existing exports of data: object cannot be re-sized")); } eprintln!("extend not exported, proceeding");
                                if let PyObject::ByteArray(bytes) = &mut *args[0].borrow_mut() {
                                let idx = if args.len() > 1 {
                                    let i = args[1].as_i64().ok_or_else(|| {
                                        PyError::type_error("pop index must be an integer")
                                    })?;
                                    let len = bytes.len() as i64;
                                    if i < 0 {
                                        len + i
                                    } else {
                                        i
                                    }
                                } else {
                                    bytes.len() as i64 - 1
                                };
                                if idx < 0 || idx >= bytes.len() as i64 {
                                    return Err(PyError::index_error("pop index out of range"));
                                }
                                let val = bytes.remove(idx as usize);
                                Ok(PyObjectRef::imm(PyObject::Int(BigInt::from(val))))
                            } else {
                                Err(PyError::runtime_error("pop on non-bytearray"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__getitem__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__getitem__".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error("__getitem__() requires an index"));
                            }
                            py_getitem(&args[0], &args[1])
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__setitem__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__setitem__".to_string(),
                        func: |args| {
                            if args.len() < 3 {
                                return Err(PyError::type_error(
                                    "__setitem__() requires an index and value",
                                ));
                            }
                            py_setitem(&args[0], &args[1], args[2].clone())?;
                            Ok(py_none())
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__len__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__len__".to_string(),
                        func: |args| {
                            let b = args[0].borrow();
                            if let PyObject::ByteArray(bytes) = &*b {
                                Ok(py_int(bytes.len() as i64))
                            } else {
                                Err(PyError::runtime_error("__len__ on non-bytearray"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__str__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__str__".to_string(),
                        func: |args| {
                            if let PyObject::ByteArray(bytes) = &*args[0].borrow() {
                                let hex: String =
                                    bytes.iter().map(|b| format!("{:02x}", b)).collect();
                                Ok(py_str(&hex))
                            } else {
                                Err(PyError::runtime_error("__str__ on non-bytearray"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__sizeof__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__sizeof__".to_string(),
                        func: |args| {
                            if let PyObject::ByteArray(bytes) = &*args[0].borrow() {
                                Ok(py_int(33 + bytes.len() as i64))
                            } else {
                                Err(PyError::runtime_error("__sizeof__ on non-bytearray"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "removeprefix" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "removeprefix".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "removeprefix() takes exactly 1 argument",
                                ));
                            }
                            if let PyObject::ByteArray(b) = &*args[0].borrow() {
                                let prefix = args[1].borrow();
                                if let PyObject::Bytes(p) = &*prefix {
                                    if b.starts_with(p.as_slice()) {
                                        Ok(PyObjectRef::imm(PyObject::ByteArray(
                                            b[p.len()..].to_vec(),
                                        )))
                                    } else {
                                        Ok(PyObjectRef::imm(PyObject::ByteArray(b.clone())))
                                    }
                                } else if let PyObject::ByteArray(p) = &*prefix {
                                    if b.starts_with(p.as_slice()) {
                                        Ok(PyObjectRef::imm(PyObject::ByteArray(
                                            b[p.len()..].to_vec(),
                                        )))
                                    } else {
                                        Ok(PyObjectRef::imm(PyObject::ByteArray(b.clone())))
                                    }
                                } else {
                                    Err(PyError::type_error(
                                        "removeprefix() argument must be bytes-like",
                                    ))
                                }
                            } else {
                                Err(PyError::runtime_error("removeprefix on non-bytearray"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "removesuffix" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "removesuffix".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "removesuffix() takes exactly 1 argument",
                                ));
                            }
                            if let PyObject::ByteArray(b) = &*args[0].borrow() {
                                let suffix = args[1].borrow();
                                if let PyObject::Bytes(s) = &*suffix {
                                    if b.ends_with(s.as_slice()) {
                                        Ok(PyObjectRef::imm(PyObject::ByteArray(
                                            b[..b.len() - s.len()].to_vec(),
                                        )))
                                    } else {
                                        Ok(PyObjectRef::imm(PyObject::ByteArray(b.clone())))
                                    }
                                } else if let PyObject::ByteArray(s) = &*suffix {
                                    if b.ends_with(s.as_slice()) {
                                        Ok(PyObjectRef::imm(PyObject::ByteArray(
                                            b[..b.len() - s.len()].to_vec(),
                                        )))
                                    } else {
                                        Ok(PyObjectRef::imm(PyObject::ByteArray(b.clone())))
                                    }
                                } else {
                                    Err(PyError::type_error(
                                        "removesuffix() argument must be bytes-like",
                                    ))
                                }
                            } else {
                                Err(PyError::runtime_error("removesuffix on non-bytearray"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    // Delegate to `bytes`'s implementation — see
                    // `bytearray_delegate`'s doc comment above.
                    "startswith" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "startswith".to_string(),
                        func: |args| bytearray_delegate("startswith", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "endswith" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "endswith".to_string(),
                        func: |args| bytearray_delegate("endswith", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "find" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "find".to_string(),
                        func: |args| bytearray_delegate("find", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "rfind" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "rfind".to_string(),
                        func: |args| bytearray_delegate("rfind", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "index" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "index".to_string(),
                        func: |args| bytearray_delegate("index", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "rindex" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "rindex".to_string(),
                        func: |args| bytearray_delegate("rindex", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "count" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "count".to_string(),
                        func: |args| bytearray_delegate("count", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "replace" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "replace".to_string(),
                        func: |args| bytearray_delegate("replace", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "split" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "split".to_string(),
                        func: |args| bytearray_delegate("split", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "rsplit" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "rsplit".to_string(),
                        func: |args| bytearray_delegate("rsplit", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "strip" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "strip".to_string(),
                        func: |args| bytearray_delegate("strip", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "lstrip" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "lstrip".to_string(),
                        func: |args| bytearray_delegate("lstrip", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "rstrip" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "rstrip".to_string(),
                        func: |args| bytearray_delegate("rstrip", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "join" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "join".to_string(),
                        func: |args| bytearray_delegate("join", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "upper" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "upper".to_string(),
                        func: |args| bytearray_delegate("upper", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "lower" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "lower".to_string(),
                        func: |args| bytearray_delegate("lower", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "title" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "title".to_string(),
                        func: |args| bytearray_delegate("title", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "capitalize" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "capitalize".to_string(),
                        func: |args| bytearray_delegate("capitalize", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "swapcase" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "swapcase".to_string(),
                        func: |args| bytearray_delegate("swapcase", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "isalpha" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "isalpha".to_string(),
                        func: |args| bytearray_delegate("isalpha", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "isdigit" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "isdigit".to_string(),
                        func: |args| bytearray_delegate("isdigit", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "isalnum" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "isalnum".to_string(),
                        func: |args| bytearray_delegate("isalnum", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "isspace" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "isspace".to_string(),
                        func: |args| bytearray_delegate("isspace", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "isupper" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "isupper".to_string(),
                        func: |args| bytearray_delegate("isupper", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "islower" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "islower".to_string(),
                        func: |args| bytearray_delegate("islower", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "istitle" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "istitle".to_string(),
                        func: |args| bytearray_delegate("istitle", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "partition" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "partition".to_string(),
                        func: |args| bytearray_delegate("partition", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "rpartition" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "rpartition".to_string(),
                        func: |args| bytearray_delegate("rpartition", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "splitlines" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "splitlines".to_string(),
                        func: |args| bytearray_delegate("splitlines", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "expandtabs" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "expandtabs".to_string(),
                        func: |args| bytearray_delegate("expandtabs", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "zfill" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "zfill".to_string(),
                        func: |args| bytearray_delegate("zfill", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "ljust" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "ljust".to_string(),
                        func: |args| bytearray_delegate("ljust", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "rjust" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "rjust".to_string(),
                        func: |args| bytearray_delegate("rjust", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "center" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "center".to_string(),
                        func: |args| bytearray_delegate("center", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "translate" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "translate".to_string(),
                        func: |args| bytearray_delegate("translate", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "decode" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "decode".to_string(),
                        func: |args| bytearray_delegate("decode", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "hex" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "hex".to_string(),
                        func: |args| bytearray_delegate("hex", args),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "clear" => {
                        Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                            name: "clear".to_string(),
                            func: |args| {
                                if crate::object::is_bytearray_exported(&args[0]) {
                                    return Err(PyError::buffer_error("Existing exports of data: object cannot be re-sized"));
                                }
                                if let PyObject::ByteArray(b) = &mut *args[0].borrow_mut() {
                                    b.clear();
                                    Ok(py_none())
                                } else {
                                    Err(PyError::runtime_error("clear on non-bytearray"))
                                }
                            },
                            self_obj: PyObjectRef::new(PyObject::None),
                        }))
                    }
                    "__buffer__" => {
                        Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                            name: "__buffer__".to_string(),
                            func: |args| {
                                if args.len() < 2 {
                                    return Err(PyError::type_error("__buffer__() takes exactly one argument"));
                                }
                                let flags = crate::object::extract_flags_for_buffer(&args[1])?;
                                crate::object::check_buffer_flags(flags)?;
                                let len = if let PyObject::ByteArray(b) = &*args[0].borrow() { b.len() } else {
                                    if let Some(backing) = crate::object::native_backing_of(&args[0]) {
                                        if let PyObject::ByteArray(b) = &*backing.borrow() { b.len() } else { 0 }
                                    } else { 0 }
                                };
                                let view = PyObjectRef::new(PyObject::MemoryView { source: args[0].clone(), format: "B".to_string(), shape: vec![len], itemsize: 1, offset: 0, readonly: false, released: false });
                                crate::object::track_view_exporter(&view, args[0].clone());
                                crate::object::increment_bytearray_export(&args[0]);
                                Ok(view)
                            },
                            self_obj: PyObjectRef::new(PyObject::None),
                        }))
                    }
                    "__release_buffer__" => {
                        Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                            name: "__release_buffer__".to_string(),
                            func: |args| {
                                if args.len() < 2 {
                                    return Err(PyError::type_error("__release_buffer__() takes exactly one argument"));
                                }
                                let view = &args[1];
                                if let PyObject::MemoryView { released, .. } = &*view.borrow() {
                                    if *released {
                                        return Err(PyError::value_error("buffer already released"));
                                    }
                                }
                                // Check if view's source matches self's backing - if not, raise ValueError (mismatched buffer)
                                {
                                    let view_source = {
                                        let b = view.borrow();
                                        if let PyObject::MemoryView { source, .. } = &*b {
                                            Some(source.clone())
                                        } else { None }
                                    };
                                    if let Some(vs) = view_source {
                                        let self_backing = crate::object::native_backing_of(&args[0]).unwrap_or_else(|| args[0].clone());
                                        let is_same = vs.is(&self_backing) || vs.is(&args[0]);
                                        // Also check via bytes equality? For bytes vs bytearray, they are different objects, so not same
                                        if !is_same {
                                            // For bytearray subclass that returned a different buffer (e.g. bytes), mismatch should raise ValueError
                                            // Check if view's source is bytearray/bytes of same length but different object -> raise
                                            // We check if view's source type is Bytes vs ByteArray mismatch
                                            let vs_is_bytes = matches!(&*vs.borrow(), PyObject::Bytes(_));
                                            let self_is_bytearray = matches!(&*self_backing.borrow(), PyObject::ByteArray(_));
                                            if vs_is_bytes || self_is_bytearray {
                                                // If sources are not same object, raise
                                                return Err(PyError::value_error("buffer mismatch: view not from this object"));
                                            }
                                        }
                                    }
                                }
                                {
                                    let mut b = view.borrow_mut();
                                    if let PyObject::MemoryView { released, .. } = &mut *b {
                                        *released = true;
                                    }
                                }
                                Ok(py_none())
                            },
                            self_obj: PyObjectRef::new(PyObject::None),
                        }))
                    }
                    _ => Err(PyError::attribute_error(format!(
                        "'bytearray' object has no attribute '{}'",
                        name
                    ))),
                }
            }
            PyObject::Str(_s) => return str1::get(self, name),
            // dict-protocol methods on the live `globals()` view — same
            // surface as `dict` below, but operating on the backing
            // `Rc<RefCell<HashMap<StrId, PyObjectRef>>>` so mutators
            // (`update`/`setdefault`/`pop`/`clear`) stay visible to
            // LOAD_GLOBAL.
            PyObject::Globals(_) => {
                fn globals_key<'a>(args: &'a [PyObjectRef], i: usize) -> Result<crate::interner::StrId, PyError> {
                    match &*args[i].borrow() {
                        PyObject::Str(s) => Ok(crate::interner::intern(s.as_str())),
                        _ => Err(PyError::key_error(args[i].str())),
                    }
                }
                match name {
                    "keys" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "keys".to_string(),
                        func: |args| {
                            if let PyObject::Globals(g) = &*args[0].borrow() {
                                let keys: Vec<PyObjectRef> = g
                                    .borrow()
                                    .keys()
                                    .map(|k| py_str(crate::interner::lookup_str(*k)))
                                    .collect();
                                Ok(py_list(keys))
                            } else if let PyObject::Dict(_d) = &*args[0].borrow() {
                                return Ok(crate::object::pydict::make_dict_view(
                                    "dict_keys",
                                    args[0].clone(),
                                ));
                            } else {
                                Err(PyError::runtime_error("keys on non-dict"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "values" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "values".to_string(),
                        func: |args| {
                            if let PyObject::Dict(_d) = &*args[0].borrow() {
                                return Ok(crate::object::pydict::make_dict_view(
                                    "dict_values",
                                    args[0].clone(),
                                ));
                            } else if let PyObject::Globals(g) = &*args[0].borrow() {
                                let values: Vec<PyObjectRef> =
                                    g.borrow().values().cloned().collect();
                                Ok(py_list(values))
                            } else {
                                Err(PyError::runtime_error("values on non-dict"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "items" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "items".to_string(),
                        func: |args| {
                            if let PyObject::Dict(_d) = &*args[0].borrow() {
                                return Ok(crate::object::pydict::make_dict_view(
                                    "dict_items",
                                    args[0].clone(),
                                ));
                            } else if let PyObject::Globals(g) = &*args[0].borrow() {
                                let items: Vec<PyObjectRef> = g
                                    .borrow()
                                    .iter()
                                    .map(|(k, v)| {
                                        py_tuple(vec![
                                            py_str(crate::interner::lookup_str(*k)),
                                            v.clone(),
                                        ])
                                    })
                                    .collect();
                                Ok(py_list(items))
                            } else {
                                Err(PyError::runtime_error("items on non-dict"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "get" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "get".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error("get() takes at least 1 argument"));
                            }
                            if let PyObject::Globals(g) = &*args[0].borrow() {
                                let key = globals_key(args, 1)?;
                                Ok(g.borrow().get(&key).cloned().unwrap_or_else(|| {
                                    if args.len() > 2 {
                                        args[2].clone()
                                    } else {
                                        py_none()
                                    }
                                }))
                            } else if let PyObject::Dict(d) = &*args[0].borrow() {
                                Ok(d.get(&args[1])?.unwrap_or_else(|| {
                                    if args.len() > 2 {
                                        args[2].clone()
                                    } else {
                                        py_none()
                                    }
                                }))
                            } else {
                                Err(PyError::runtime_error("get on non-dict"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "setdefault" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "setdefault".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "setdefault() takes at least 1 argument",
                                ));
                            }
                            let default = if args.len() > 2 {
                                args[2].clone()
                            } else {
                                py_none()
                            };
                            if let PyObject::Globals(g) = &*args[0].borrow() {
                                let key = globals_key(args, 1)?;
                                let mut map = g.borrow_mut();
                                if let Some(v) = map.get(&key) {
                                    return Ok(v.clone());
                                }
                                map.insert(key, default.clone());
                                Ok(default)
                            } else if matches!(&*args[0].borrow(), PyObject::Dict(_)) {
                                let mut b = args[0].borrow_mut();
                                if let PyObject::Dict(d) = &mut *b {
                                    match d.get(&args[1])? {
                                        Some(v) => Ok(v),
                                        None => {
                                            d.set(args[1].clone(), default.clone())?;
                                            Ok(default)
                                        }
                                    }
                                } else {
                                    unreachable!()
                                }
                            } else {
                                Err(PyError::runtime_error("setdefault on non-dict"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "pop" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "pop".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error("pop() takes at least 1 argument"));
                            }
                            if let PyObject::Globals(g) = &*args[0].borrow() {
                                let key = globals_key(args, 1)?;
                                match g.borrow_mut().remove(&key) {
                                    Some(v) => Ok(v),
                                    None => {
                                        if args.len() > 2 {
                                            Ok(args[2].clone())
                                        } else {
                                            Err(PyError::key_error(args[1].str()))
                                        }
                                    }
                                }
                            } else if matches!(&*args[0].borrow(), PyObject::Dict(_)) {
                                let mut b = args[0].borrow_mut();
                                if let PyObject::Dict(d) = &mut *b {
                                    match d.remove(&args[1]) {
                                        Ok(val) => Ok(val),
                                        Err(_) if args.len() > 2 => Ok(args[2].clone()),
                                        Err(e) => Err(e),
                                    }
                                } else {
                                    unreachable!()
                                }
                            } else {
                                Err(PyError::runtime_error("pop on non-dict"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "popitem" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "popitem".to_string(),
                        func: |args| {
                            if args.len() > 1 {
                                return Err(PyError::type_error(format!(
                                    "dict.popitem() takes no arguments ({} given)",
                                    args.len() - 1
                                )));
                            }
                            if let PyObject::Globals(g) = &*args[0].borrow() {
                                let mut map = g.borrow_mut();
                                let first = map.iter().next().map(|(k, v)| {
                                    (
                                        *k,
                                        py_str(crate::interner::lookup_str(*k)),
                                        v.clone(),
                                    )
                                });
                                if let Some((key, kobj, v)) = first {
                                    map.remove(&key);
                                    Ok(py_tuple(vec![kobj, v]))
                                } else {
                                    Err(PyError::key_error(
                                        "popitem(): dictionary is empty".to_string(),
                                    ))
                                }
                            } else if matches!(&*args[0].borrow(), PyObject::Dict(_)) {
                                let mut b = args[0].borrow_mut();
                                if let PyObject::Dict(d) = &mut *b {
                                    let items = d.items();
                                    if items.is_empty() {
                                        return Err(PyError::key_error(
                                            "popitem(): dictionary is empty".to_string(),
                                        ));
                                    }
                                    let (k, v) = items.into_iter().last().unwrap();
                                    d.remove(&k)?;
                                    Ok(py_tuple(vec![k, v]))
                                } else {
                                    unreachable!()
                                }
                            } else {
                                Err(PyError::runtime_error("popitem on non-dict"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "clear" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "clear".to_string(),
                        func: |args| {
                            if let PyObject::Globals(g) = &*args[0].borrow() {
                                g.borrow_mut().clear();
                                Ok(py_none())
                            } else if matches!(&*args[0].borrow(), PyObject::Dict(_)) {
                                let mut b = args[0].borrow_mut();
                                if let PyObject::Dict(d) = &mut *b {
                                    d.clear();
                                }
                                Ok(py_none())
                            } else {
                                Err(PyError::runtime_error("clear on non-dict"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "copy" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "copy".to_string(),
                        func: |args| {
                            if let PyObject::Globals(g) = &*args[0].borrow() {
                                let mut d = crate::object::PyDict::new();
                                for (k, v) in g.borrow().iter() {
                                    d.set(py_str(crate::interner::lookup_str(*k)), v.clone())?;
                                }
                                Ok(PyObjectRef::new(PyObject::Dict(Box::new(d))))
                            } else if let PyObject::Dict(src) = &*args[0].borrow() {
                                Ok(PyObjectRef::new(PyObject::Dict(Box::new((**src).clone()))))
                            } else {
                                Err(PyError::runtime_error("copy on non-dict"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "update" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "update".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "update() takes at least 1 argument",
                                ));
                            }
                            if let PyObject::Globals(g) = &*args[0].borrow() {
                                let src = args[1].borrow();
                                match &*src {
                                    PyObject::Dict(d) => {
                                        let mut map = g.borrow_mut();
                                        for (k, v) in d.items() {
                                            if let PyObject::Str(s) = &*k.borrow() {
                                                map.insert(
                                                    crate::interner::intern(s.as_str()),
                                                    v,
                                                );
                                            }
                                        }
                                        Ok(py_none())
                                    }
                                    PyObject::Globals(other) => {
                                        let pairs: Vec<(String, PyObjectRef)> = other
                                            .borrow()
                                            .iter()
                                            .map(|(k, v)| {
                                                (
                                                    crate::interner::lookup_str(*k).to_string(),
                                                    v.clone(),
                                                )
                                            })
                                            .collect();
                                        drop(src);
                                        let mut map = g.borrow_mut();
                                        for (k, v) in pairs {
                                            map.insert(crate::interner::intern(&k), v);
                                        }
                                        Ok(py_none())
                                    }
                                    _ => Err(PyError::type_error(
                                        "update() argument must be a dict".to_string(),
                                    )),
                                }
                            } else if matches!(&*args[0].borrow(), PyObject::Dict(_)) {
                                let mut db = args[0].borrow_mut();
                                let dst = match &mut *db {
                                    PyObject::Dict(d) => d,
                                    _ => unreachable!(),
                                };
                                let src = args[1].borrow();
                                match &*src {
                                    PyObject::Dict(d) => {
                                        for (k, v) in d.items() {
                                            dst.set(k.clone(), v)?;
                                        }
                                        Ok(py_none())
                                    }
                                    PyObject::Globals(other) => {
                                        let pairs: Vec<(PyObjectRef, PyObjectRef)> = other
                                            .borrow()
                                            .iter()
                                            .map(|(k, v)| {
                                                (
                                                    py_str(crate::interner::lookup_str(*k)),
                                                    v.clone(),
                                                )
                                            })
                                            .collect();
                                        drop(src);
                                        for (k, v) in pairs {
                                            dst.set(k, v)?;
                                        }
                                        Ok(py_none())
                                    }
                                    _ => Err(PyError::type_error(
                                        "update() argument must be a dict".to_string(),
                                    )),
                                }
                            } else {
                                Err(PyError::runtime_error("update on non-dict"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    _ => Err(PyError::attribute_error(format!(
                        "'dict' object has no attribute '{}'",
                        name
                    ))),
                }
            }
            PyObject::Dict(_d) => {
                match name {
                    "keys" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "keys".to_string(),
                        func: |args| {
                            let d = args[0].borrow();
                            if let PyObject::Dict(dict) = &*d {
                                return Ok(crate::object::pydict::make_dict_view(
                                    "dict_keys",
                                    args[0].clone(),
                                ));
                            } else if let PyObject::Globals(g) = &*d {
                                let keys: Vec<PyObjectRef> = g
                                    .borrow()
                                    .keys()
                                    .map(|k| py_str(crate::interner::lookup_str(*k)))
                                    .collect();
                                Ok(py_list(keys))
                            } else {
                                Err(PyError::runtime_error("keys on non-dict"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "values" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "values".to_string(),
                        func: |args| {
                            let d = args[0].borrow();
                            if let PyObject::Dict(dict) = &*d {
                                return Ok(crate::object::pydict::make_dict_view(
                                    "dict_values",
                                    args[0].clone(),
                                ));
                            } else if let PyObject::Globals(g) = &*d {
                                let values: Vec<PyObjectRef> =
                                    g.borrow().values().cloned().collect();
                                Ok(py_list(values))
                            } else {
                                Err(PyError::runtime_error("values on non-dict"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "items" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "items".to_string(),
                        func: |args| {
                            let d = args[0].borrow();
                            if let PyObject::Dict(_dict) = &*d {
                                return Ok(crate::object::pydict::make_dict_view(
                                    "dict_items",
                                    args[0].clone(),
                                ));
                            } else if let PyObject::Globals(g) = &*d {
                                let items: Vec<PyObjectRef> = g
                                    .borrow()
                                    .iter()
                                    .map(|(k, v)| {
                                        py_tuple(vec![
                                            py_str(crate::interner::lookup_str(*k)),
                                            v.clone(),
                                        ])
                                    })
                                    .collect();
                                Ok(py_list(items))
                            } else {
                                Err(PyError::runtime_error("items on non-dict"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "get" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "get".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error("get() takes at least 1 argument"));
                            }
                            let dict = &*args[0].borrow();
                            if let PyObject::Dict(d) = dict {
                                Ok(d.get(&args[1])?.unwrap_or_else(|| {
                                    if args.len() > 2 {
                                        args[2].clone()
                                    } else {
                                        py_none()
                                    }
                                }))
                            } else if let PyObject::Globals(g) = dict {
                                let key = match &*args[1].borrow() {
                                    PyObject::Str(s) => crate::interner::intern(s.as_str()),
                                    _ => return Ok(py_none()),
                                };
                                Ok(g.borrow().get(&key).cloned().unwrap_or_else(|| {
                                    if args.len() > 2 {
                                        args[2].clone()
                                    } else {
                                        py_none()
                                    }
                                }))
                            } else {
                                Err(PyError::runtime_error("get on non-dict"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "pop" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "pop".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error("pop() takes at least 1 argument"));
                            }
                            if let PyObject::Dict(d) = &mut *args[0].borrow_mut() {
                                match d.remove(&args[1]) {
                                    Ok(val) => Ok(val),
                                    Err(_) if args.len() > 2 => Ok(args[2].clone()),
                                    Err(e) => Err(e),
                                }
                            } else {
                                Err(PyError::runtime_error("pop on non-dict"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "popitem" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "popitem".to_string(),
                        func: |args| {
                            // Real `dict.popitem()` takes NO arguments at
                            // all (unlike `OrderedDict.popitem(last=True)`,
                            // a genuinely different method on a different
                            // type) — this silently accepted and ignored
                            // any extra positional argument instead of
                            // raising, confirmed via CPython's own
                            // `test_dict.py`/`mapping_tests.py::
                            // test_popitem`, which explicitly checks
                            // `assertRaises(TypeError, d.popitem, 42)`.
                            if args.len() > 1 {
                                return Err(PyError::type_error(format!(
                                    "dict.popitem() takes no arguments ({} given)",
                                    args.len() - 1
                                )));
                            }
                            if let PyObject::Dict(d) = &mut *args[0].borrow_mut() {
                                let items = d.items();
                                if items.is_empty() {
                                    return Err(PyError::key_error(
                                        "popitem(): dictionary is empty",
                                    ));
                                }
                                let (k, v) = items.into_iter().last().unwrap();
                                d.remove(&k)?;
                                Ok(py_tuple(vec![k, v]))
                            } else {
                                Err(PyError::runtime_error("popitem on non-dict"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "clear" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "clear".to_string(),
                        func: |args| {
                            if let PyObject::Dict(d) = &mut *args[0].borrow_mut() {
                                d.clear();
                                Ok(py_none())
                            } else {
                                Err(PyError::runtime_error("clear on non-dict"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "update" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "update".to_string(),
                        func: |args| {
                            if args.is_empty() {
                                return Err(PyError::type_error(
                                    "update() takes at least 1 argument",
                                ));
                            }
                            let self_obj = args[0].clone();
                            // Matches CPython's real dict.update(): accepts another
                            // dict, any mapping-protocol object (has .keys()), or an
                            // iterable of (key, value) pairs. A trailing kwargs dict
                            // (from `d.update(x, k=v)`) is just another entry here.
                            for other in &args[1..] {
                                let is_dict = matches!(&*other.borrow(), PyObject::Dict(_));
                                if is_dict {
                                    let items = if let PyObject::Dict(other_dict) = &*other.borrow()
                                    {
                                        other_dict.items()
                                    } else {
                                        unreachable!()
                                    };
                                    if let PyObject::Dict(d) = &mut *self_obj.borrow_mut() {
                                        for (k, v) in items {
                                            d.set(k, v)?;
                                        }
                                    }
                                    continue;
                                }
                                // A native-backed dict subclass (Counter, defaultdict,
                                // or any `class Foo(dict): ...`) — read straight off
                                // the native backing rather than resolving `keys`.
                                if let Some(native) = native_backing_of(other) {
                                    if let PyObject::Dict(other_dict) = &*native.borrow() {
                                        let items = other_dict.items();
                                        if let PyObject::Dict(d) = &mut *self_obj.borrow_mut() {
                                            for (k, v) in items {
                                                d.set(k, v)?;
                                            }
                                        }
                                        continue;
                                    }
                                }
                                let keys_fn = match &*other.borrow() {
                                    PyObject::Instance { typ, .. } => {
                                        lookup_dunder_via_mro(typ, "keys")
                                    }
                                    _ => None,
                                };
                                if let Some(keys_fn) = keys_fn {
                                    let keys_obj =
                                        call_bound_method(keys_fn, other.clone(), vec![])?;
                                    let it = builtin_iter(&[keys_obj])?;
                                    loop {
                                        match builtin_next(&[it.clone()]) {
                                            Ok(k) => {
                                                let v = py_getitem(other, &k)?;
                                                if let PyObject::Dict(d) =
                                                    &mut *self_obj.borrow_mut()
                                                {
                                                    d.set(k, v)?;
                                                }
                                            }
                                            Err(PyError::StopIteration) => break,
                                            Err(e) => return Err(e),
                                        }
                                    }
                                } else {
                                    let it = builtin_iter(&[other.clone()])?;
                                    loop {
                                        match builtin_next(&[it.clone()]) {
                                            Ok(pair) => {
                                                let (k, v) = match &*pair.borrow() {
                                                    PyObject::Tuple(items) | PyObject::List(items) if items.len() == 2 => {
                                                        (items[0].clone(), items[1].clone())
                                                    }
                                                    _ => return Err(PyError::type_error("cannot convert update sequence element to a sequence")),
                                                };
                                                if let PyObject::Dict(d) =
                                                    &mut *self_obj.borrow_mut()
                                                {
                                                    d.set(k, v)?;
                                                }
                                            }
                                            Err(PyError::StopIteration) => break,
                                            Err(e) => return Err(e),
                                        }
                                    }
                                }
                            }
                            Ok(py_none())
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "setdefault" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "setdefault".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "setdefault() takes at least 1 argument",
                                ));
                            }
                            let key = args[1].clone();
                            let default = if args.len() > 2 {
                                args[2].clone()
                            } else {
                                py_none()
                            };
                            // Routed through `pydict_safe_get_or_insert` — see
                            // `pydict_safe_set`'s doc comment (subscript.rs)
                            // for why this must never hold `args[0]`'s own
                            // mutable borrow across a colliding key's
                            // `.equals()` call (real CPython test:
                            // `test_dict.py`'s `test_clear_at_lookup`, which
                            // exercises this exact method).
                            //
                            // Globals receivers are Imm and wrap their own
                            // RefCell'd map — the safe-or-insert helper
                            // requires a Mut Dict wrapper and would panic.
                            if matches!(&*args[0].borrow(), PyObject::Globals(_)) {
                                let sid = crate::interner::intern(&key.str());
                                let g = match &*args[0].borrow() {
                                    PyObject::Globals(g) => g.clone(),
                                    _ => unreachable!(),
                                };
                                if let Some(existing) = g.borrow().get(&sid) {
                                    return Ok(existing.clone());
                                }
                                g.borrow_mut().insert(sid, default.clone());
                                return Ok(default);
                            }
                            pydict_safe_get_or_insert(&args[0], key, default)
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "copy" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "copy".to_string(),
                        func: |args| {
                            let d = args[0].borrow();
                            if let PyObject::Dict(dict) = &*d {
                                let mut new_dict = PyDict::new();
                                for (k, v) in dict.items() {
                                    new_dict.set(k, v)?;
                                }
                                Ok(PyObjectRef::new(PyObject::Dict(Box::new(new_dict))))
                            } else {
                                Err(PyError::runtime_error("copy on non-dict"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "fromkeys" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "fromkeys".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "fromkeys() takes at least 1 argument",
                                ));
                            }
                            let mut new_dict = PyDict::new();
                            let val = if args.len() > 2 {
                                args[2].clone()
                            } else {
                                py_none()
                            };
                            let it = builtin_iter(&[args[1].clone()])?;
                            loop {
                                match builtin_next(&[it.clone()]) {
                                    Ok(k) => {
                                        new_dict.set(k, val.clone())?;
                                    }
                                    Err(PyError::StopIteration) => break,
                                    Err(e) => return Err(e),
                                }
                            }
                            Ok(PyObjectRef::new(PyObject::Dict(Box::new(new_dict))))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__sizeof__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__sizeof__".to_string(),
                        func: |args| {
                            if let PyObject::Dict(d) = &*args[0].borrow() {
                                Ok(py_int(72 + (d.len() as i64) * 16))
                            } else {
                                Err(PyError::runtime_error("__sizeof__ on non-dict"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__contains__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__contains__".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "__contains__() takes exactly one argument",
                                ));
                            }
                            if let PyObject::Dict(d) = &*args[0].borrow() {
                                Ok(py_bool(d.contains(&args[1])?))
                            } else {
                                Err(PyError::runtime_error("__contains__ on non-dict"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    // `some_dict.__getitem__`/`__setitem__`/`__delitem__` as
                    // a bound-method REFERENCE (not called directly) — real
                    // code uses this idiom to grab a fast lookup callable
                    // (real trigger: CPython 3.14's own `_colorize.py`,
                    // `super().__setattr__('_name_to_value',
                    // name_to_value.__getitem__)`), same class of gap as
                    // `frozenset.__contains__` found earlier this session.
                    "__getitem__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__getitem__".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "__getitem__() takes exactly one argument",
                                ));
                            }
                            py_getitem(&args[0], &args[1])
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__setitem__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__setitem__".to_string(),
                        func: |args| {
                            if args.len() < 3 {
                                return Err(PyError::type_error(
                                    "__setitem__() takes exactly 2 arguments",
                                ));
                            }
                            py_setitem(&args[0], &args[1], args[2].clone())?;
                            Ok(py_none())
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__delitem__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__delitem__".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "__delitem__() takes exactly one argument",
                                ));
                            }
                            if let PyObject::Dict(d) = &mut *args[0].borrow_mut() {
                                d.remove(&args[1])?;
                                Ok(py_none())
                            } else {
                                Err(PyError::runtime_error("__delitem__ on non-dict"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "move_to_end" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "move_to_end".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "move_to_end() needs a key argument",
                                ));
                            }
                            Ok(py_none())
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__or__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__or__".to_string(),
                        func: |args| {
                            // Reachable two ways with two different argument
                            // shapes: a normal bound call (`d.__or__(x)`,
                            // rebound to `[self, other]` by the usual
                            // attribute-access path) and `py_bit_or`'s
                            // `try_dunder_binop` (`{} | d2`), which — like
                            // every other native dunder called that way —
                            // goes through `call_bound_method`'s
                            // placeholder-prepending `BuiltinMethod` arm,
                            // delivering `[None, self, other]` instead. This
                            // used to only handle the 2-arg shape, so `dict |
                            // dict` (real PEP 584 syntax) misread the
                            // placeholder as `self`, failing with a
                            // nonsensical "non-dict" error despite both
                            // operands being genuine dicts.
                            let (self_idx, other_idx) = if args.len() >= 3 {
                                (1, 2)
                            } else if args.len() == 2 {
                                (0, 1)
                            } else {
                                return Err(PyError::type_error(
                                    "__or__() takes exactly one argument",
                                ));
                            };
                            // Accept dict-subclass instances on either side
                            // (defaultdict etc.) by falling back to their
                            // keys()/__getitem__ mapping protocol.
                            fn dict_like_items(o: &PyObjectRef) -> Option<Vec<(PyObjectRef, PyObjectRef)>> {
                                let b = o.borrow();
                                if let PyObject::Dict(dd) = &*b {
                                    return Some(dd.items());
                                }
                                if b.get_attribute("keys").is_ok() {
                                    return None; // handled by caller via update-style path
                                }
                                None
                            }
                            // Reflected priority: a dict-subclass Instance
                            // (defaultdict etc.) with __ror__ must win over
                            // this native implementation.
                            if let PyObject::Instance { typ, .. } =
                                &*args[other_idx].borrow()
                            {
                                if crate::object::lookup_dunder_via_mro(typ, "__ror__")
                                    .is_some()
                                {
                                    let ror = crate::object::lookup_dunder_via_mro(
                                        typ,
                                        "__ror__",
                                    )
                                    .unwrap();
                                    return crate::object::call_bound_method(
                                        ror,
                                        args[other_idx].clone(),
                                        vec![args[self_idx].clone()],
                                    );
                                }
                            }
                            // Accept list/tuple of (k, v) pairs too:
                            // `defaultdict |= [(1,'a'), ...]` is real CPython
                            // dict.__ior__ semantics (in-place update).
                            let other_is_pairs = matches!(
                                &*args[other_idx].borrow(),
                                PyObject::List(_) | PyObject::Tuple(_)
                            );
                            let other_ok = matches!(&*args[other_idx].borrow(), PyObject::Dict(_))
                                || args[other_idx].borrow().get_attribute("keys").is_ok()
                                || other_is_pairs;
                            if !other_ok {
                                return Err(PyError::type_error(
                                    "__or__() argument must be a dict",
                                ));
                            }
                            // Build result: start from self's mapping view.
                            let self_obj_clone = args[self_idx].clone();
                            let mut new_dict = PyDict::new();
                            if let PyObject::Dict(dict) = &*self_obj_clone.borrow() {
                                for (k, v) in dict.items() {
                                    new_dict.set(k.clone(), v.clone())?;
                                }
                            } else {
                                // dict subclass instance: read via its own
                                // mapping protocol (keys + getitem).
                                let keys_m = self_obj_clone.borrow().get_attribute("keys")?;
                                let keys_it = crate::object::builtin_iter(&[crate::object::call_bound_method(keys_m, self_obj_clone.clone(), vec![])?])?;
                                loop {
                                    match crate::object::builtin_next(&[keys_it.clone()]) {
                                        Ok(k) => {
                                            let v = crate::object::py_getitem(&self_obj_clone, &k)?;
                                            new_dict.set(k, v)?;
                                        }
                                        Err(crate::object::PyError::StopIteration) => break,
                                        Err(e) => return Err(e),
                                    }
                                }
                            }
                            // Then merge other: prefer its items for dup keys.
                            if let PyObject::Dict(other_dict) = &*args[other_idx].borrow() {
                                for (k, v) in other_dict.items() {
                                    new_dict.set(k.clone(), v.clone())?;
                                }
                            } else {
                                let keys_m = args[other_idx].borrow().get_attribute("keys")?;
                                let keys_it = crate::object::builtin_iter(&[crate::object::call_bound_method(keys_m, args[other_idx].clone(), vec![])?])?;
                                loop {
                                    match crate::object::builtin_next(&[keys_it.clone()]) {
                                        Ok(k) => {
                                            let v = crate::object::py_getitem(&args[other_idx], &k)?;
                                            new_dict.set(k, v)?;
                                        }
                                        Err(crate::object::PyError::StopIteration) => break,
                                        Err(e) => return Err(e),
                                    }
                                }
                            }
                            let _ = dict_like_items; // (kept for reference)
                            Ok(PyObjectRef::new(PyObject::Dict(Box::new(new_dict))))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__len__" => {
                        let len = _d.len() as i64;
                        Ok(PyObjectRef::new(PyObject::Closure(Rc::new(
                            move |_args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                                Ok(py_int(len))
                            },
                        ))))
                    }
                    "__iter__" => {
                        let keys: Vec<PyObjectRef> = _d.keys();
                        Ok(PyObjectRef::new(PyObject::Closure(Rc::new(
                            move |_args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                                Ok(py_list(keys.clone()))
                            },
                        ))))
                    }
                    "__contains__" => {
                        let d_clone = _d.clone();
                        Ok(PyObjectRef::new(PyObject::Closure(Rc::new(
                            move |args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                                if args.is_empty() {
                                    return Err(PyError::type_error("__contains__() takes exactly 1 argument"));
                                }
                                Ok(py_bool(d_clone.contains(&args[0])?))
                            },
                        ))))
                    }
                    _ => Err(PyError::attribute_error(format!(
                        "'dict' object has no attribute '{}'",
                        name
                    ))),
                }
            }
            PyObject::Set(_s) => return set::get(self, name),
            PyObject::Function(ref f) => {
                let func_name = &f.code.name;
                let dict = &f.dict;
                match name {
                    "__name__" => Ok(dict
                        .get("__name__")
                        .cloned()
                        .unwrap_or(py_str(crate::interner::lookup_str(*func_name)))),
                    "__qualname__" => Ok(dict
                        .get("__qualname__")
                        .cloned()
                        .unwrap_or(py_str(crate::interner::lookup_str(*func_name)))),
                    "name" => Ok(dict
                        .get("name")
                        .cloned()
                        .unwrap_or(py_str(crate::interner::lookup_str(*func_name)))),
                    "__doc__" => Ok(dict.get("__doc__").cloned().unwrap_or(py_none())),
                    "__code__" => Ok(dict.get("__code__").cloned().unwrap_or(py_none())),
                    "__globals__" => Ok(dict.get("__globals__").cloned().unwrap_or(py_none())),
                    // Real `__defaults__`/`__kwdefaults__` introspection —
                    // was ALWAYS `None` regardless of the function's real
                    // signature (only reflected a value if user code
                    // explicitly assigned `f.__defaults__ = ...` by hand),
                    // even though the real default VALUES are already
                    // sitting right here on `f.defaults` (populated by
                    // `MAKE_FUNCTION`, which appends kwonly defaults after
                    // positional ones — see its own doc comment, `vm.rs`).
                    // `__kwdefaults__` additionally needs the kwonly
                    // parameter NAMES, which live in `varnames` right after
                    // the positional ones (`varnames[arg_count..][..
                    // kwonlyarg_count]` — standard CPython varnames layout).
                    // Missing entirely broke `test_keywordonlyarg.py::
                    // testKwDefaults` (`AttributeError` instead of a real
                    // dict).
                    "__defaults__" => {
                        if let Some(v) = dict.get("__defaults__") {
                            return Ok(v.clone());
                        }
                        let kwonly_with_default =
                            f.code.kwonly_defaults_mask.iter().filter(|&&b| b).count();
                        let pos_count = f.defaults.len().saturating_sub(kwonly_with_default);
                        if pos_count == 0 {
                            Ok(py_none())
                        } else {
                            Ok(py_tuple(f.defaults[..pos_count].to_vec()))
                        }
                    }
                    "__kwdefaults__" => {
                        if let Some(v) = dict.get("__kwdefaults__") {
                            return Ok(v.clone());
                        }
                        let kwonly_with_default =
                            f.code.kwonly_defaults_mask.iter().filter(|&&b| b).count();
                        if kwonly_with_default == 0 {
                            return Ok(py_none());
                        }
                        let pos_count = f.defaults.len().saturating_sub(kwonly_with_default);
                        let mut kw_d = PyDict::new();
                        let mut value_idx = pos_count;
                        for (i, &has_default) in f.code.kwonly_defaults_mask.iter().enumerate() {
                            if has_default {
                                if let Some(&name_id) = f.code.varnames.get(f.code.arg_count + i) {
                                    let arg_name = crate::interner::lookup_str(name_id);
                                    if let Some(val) = f.defaults.get(value_idx) {
                                        let _ = kw_d.set(py_str(arg_name), val.clone());
                                    }
                                }
                                value_idx += 1;
                            }
                        }
                        Ok(PyObjectRef::new(PyObject::Dict(Box::new(kw_d))))
                    }
                    "__closure__" => Ok(dict.get("__closure__").cloned().unwrap_or(py_none())),
                    "__module__" => Ok(dict.get("__module__").cloned().unwrap_or(py_none())),
                    "__annotations__" => {
                        // PEP 649: calling `__annotate__` lazily computes the
                        // annotations dict (undefined names fail only on
                        // first access). Cache per function (keyed by the
                        // __annotate__ closure's identity, or the function's
                        // own object address for the no-annotation empty
                        // dict) so repeated access returns the SAME dict —
                        // test_decorators asserts `func.__annotations__ is
                        // func.__annotations__`.
                        if let Some(annotate) = dict.get_str("__annotate__").cloned() {
                            // The decorator may have explicitly set
                            // `wrapper.__annotate__ = None` (reprlib's
                            // recursive_repr does) — None means "no lazy
                            // annotations", not a callable to invoke.
                            if !matches!(&*annotate.borrow(), PyObject::None) {
                                let key = annotate.get_id();
                                if let Some(cached) =
                                    ANN_CACHE.with(|c| c.borrow().get(&key).cloned())
                                {
                                    return Ok(cached);
                                }
                                let result = crate::object::call_function_disposable(
                                    &annotate,
                                    vec![],
                                    vec![],
                                )?;
                                ANN_CACHE.with(|c| c.borrow_mut().insert(key, result.clone()));
                                return Ok(result);
                            }
                        }
                        // No `__annotate__`: every annotation-less function
                        // shares ONE empty dict, so
                        // `func1.__annotations__ is func2.__annotations__`
                        // (test_reprlib::test_assigned_attributes asserts
                        // this across a wrapped function pair).
                        thread_local! {
                            static EMPTY_ANN: std::cell::RefCell<Option<PyObjectRef>> =
                                const { std::cell::RefCell::new(None) };
                        }
                        let empty = EMPTY_ANN.with(|c| {
                            let mut opt = c.borrow_mut();
                            if opt.is_none() {
                                *opt = Some(crate::object::py_dict());
                            }
                            opt.clone().unwrap()
                        });
                        Ok(empty)
                    }
                    // `func.__dict__` — every custom attribute set on a
                    // function (`f.custom = 1`) already lands in this same
                    // `dict` (see `set_attribute`'s `PyObject::Function`
                    // arm), but reading `__dict__` itself back out as a
                    // real dict was missing (`AttributeError`). Real
                    // trigger: CPython's own `test_funcattrs.py`-style
                    // checks of `f.__dict__`. Excludes the dunder slots
                    // above (`__name__`/`__doc__`/etc.) since real Python's
                    // `func.__dict__` only ever holds USER-set attributes,
                    // not those dedicated descriptor slots.
                    "__dict__" => {
                        let mut pd = PyDict::new();
                        for (k, v) in dict.iter() {
                            if k.starts_with("__") && k.ends_with("__") {
                                continue;
                            }
                            pd.set(py_str(k), v.clone())?;
                        }
                        Ok(PyObjectRef::new(PyObject::Dict(Box::new(pd))))
                    }
                    _ => dict.get_str(&name).cloned().ok_or_else(|| {
                        PyError::attribute_error(format!(
                            "'function' object has no attribute '{}'",
                            name
                        ))
                    }),
                }
            }
            PyObject::BoundMethod { func, self_obj } => {
                match name {
                    "__func__" => Ok(func.clone()),
                    "__self__" => Ok(self_obj.clone()),
                    // A real Python bound method proxies any attribute not
                    // found on the method object itself through to the
                    // underlying function (`__func__`) — this is how e.g.
                    // `SomeClass.some_classmethod.cache_clear()` reaches the
                    // functools.cache wrapper underneath the classmethod
                    // descriptor. Without this fallback, BoundMethod had no
                    // get_attribute arm at all and every such access raised
                    // "'method' object has no attribute ...".
                    //
                    // `func.get_attribute` alone (the ObjectAccess impl) does
                    // raw, unbound retrieval — it doesn't replicate LOAD_ATTR's
                    // self-binding for the result. Redo that binding here so
                    // e.g. `.cache_clear` comes back as a real bound call
                    // (self = func, the underlying cache-wrapper instance),
                    // not a plain unbound Function that would immediately hit
                    // "local variable 'self' referenced before assignment".
                    _ => {
                        let raw = func.borrow().get_attribute(name).map_err(|_| {
                            if std::env::var("RPY_DEBUG_ATTR").is_ok() {
                                let (fn_name, fn_file) = if let PyObject::Function(ref inner_f) = &*func.borrow() {
                                    let code = &inner_f.code;
                                    (code.name.to_string(), code.filename.to_string())
                                } else { ("?".to_string(), "?".to_string()) };
                                let self_kind = match &*self_obj.borrow() {
                                    PyObject::Type { name, .. } => format!("Type({})", name),
                                    other => format!("{}", other.type_name()),
                                };
                                eprintln!("BOUNDMETHOD_ATTR_FAIL: name={} func_kind={:?} fn_name={} fn_file={} self_kind={}", name, func.borrow().type_name(), fn_name, fn_file, self_kind);
                            }
                            PyError::attribute_error(format!(
                            "'method' object has no attribute '{}'", name
                        ))})?;
                        let is_instance_self = matches!(&*func.borrow(), PyObject::Instance { .. });
                        let raw_kind = {
                            let b = raw.borrow();
                            match &*b {
                                PyObject::Function { .. } if is_instance_self => 1,
                                PyObject::BuiltinFunction { .. } => 2,
                                PyObject::BuiltinMethod { .. } => 3,
                                _ => 0,
                            }
                        };
                        match raw_kind {
                            1 => Ok(PyObjectRef::imm(PyObject::BoundMethod {
                                func: raw,
                                self_obj: func.clone(),
                            })),
                            2 => {
                                let (n, f) = if let PyObject::BuiltinFunction { name: n, func: f } =
                                    &*raw.borrow()
                                {
                                    (n.clone(), *f)
                                } else {
                                    unreachable!()
                                };
                                Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                                    name: n,
                                    func: f,
                                    self_obj: func.clone(),
                                }))
                            }
                            3 => {
                                let (n, f) =
                                    if let PyObject::BuiltinMethod {
                                        name: n, func: f, ..
                                    } = &*raw.borrow()
                                    {
                                        (n.clone(), *f)
                                    } else {
                                        unreachable!()
                                    };
                                Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                                    name: n,
                                    func: f,
                                    self_obj: func.clone(),
                                }))
                            }
                            _ => Ok(raw),
                        }
                    }
                }
            }
            PyObject::Generator { .. } => return generator::get(self, name),
            PyObject::Coroutine { frame: _coro_frame } => match name {
                "send" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "send".to_string(),
                    func: coroutine_send_fallback,
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                "throw" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "throw".to_string(),
                    func: coroutine_throw_fallback,
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                "close" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "close".to_string(),
                    func: |args| {
                        let gen = args[0].borrow();
                        if let PyObject::Coroutine { frame } = &*gen {
                            if let Ok(mut frame_opt) = frame.try_borrow_mut() {
                                *frame_opt = None;
                            }
                        }
                        Ok(py_none())
                    },
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                "__await__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "__await__".to_string(),
                    func: |args| Ok(args[0].clone()),
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                "__iter__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "__iter__".to_string(),
                    func: |args| Ok(args[0].clone()),
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                "__await__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "__await__".to_string(),
                    func: |args| Ok(args[0].clone()),
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                "__anext__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "__anext__".to_string(),
                    func: |args| {
                        if let PyObject::Coroutine { .. } = &*args[0].borrow() {
                            let send_method = args[0].borrow().get_attribute("send")?;
                            let (n, f) = {
                                let b = send_method.borrow();
                                if let PyObject::BuiltinMethod { name, func, .. } = &*b {
                                    (name.clone(), *func)
                                } else {
                                    return Err(PyError::runtime_error("expected send method"));
                                }
                            };
                            let fixed = PyObjectRef::imm(PyObject::BuiltinMethod {
                                name: n,
                                func: f,
                                self_obj: args[0].clone(),
                            });
                            let mut vm = crate::vm::VirtualMachine::new();
                            vm.call_function(fixed, vec![crate::object::py_none()], vec![])
                        } else {
                            Err(PyError::runtime_error("__anext__ on non-coroutine"))
                        }
                    },
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                "__aiter__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "__aiter__".to_string(),
                    func: |args| Ok(args[0].clone()),
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                "asend" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "asend".to_string(),
                    func: |args| {
                        if let PyObject::Coroutine { .. } = &*args[0].borrow() {
                            let send_method = args[0].borrow().get_attribute("send")?;
                            let (n, f) = {
                                let b = send_method.borrow();
                                if let PyObject::BuiltinMethod { name, func, .. } = &*b {
                                    (name.clone(), *func)
                                } else {
                                    return Err(PyError::runtime_error("expected send method"));
                                }
                            };
                            let fixed = PyObjectRef::imm(PyObject::BuiltinMethod {
                                name: n,
                                func: f,
                                self_obj: args[0].clone(),
                            });
                            let val = if args.len() > 1 {
                                args[1].clone()
                            } else {
                                crate::object::py_none()
                            };
                            let mut vm = crate::vm::VirtualMachine::new();
                            vm.call_function(fixed, vec![val], vec![])
                        } else {
                            Err(PyError::runtime_error("asend on non-coroutine"))
                        }
                    },
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                "athrow" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "athrow".to_string(),
                    func: |args| {
                        if let PyObject::Coroutine { .. } = &*args[0].borrow() {
                            let throw_method = args[0].borrow().get_attribute("throw")?;
                            let (n, f) = {
                                let b = throw_method.borrow();
                                if let PyObject::BuiltinMethod { name, func, .. } = &*b {
                                    (name.clone(), *func)
                                } else {
                                    return Err(PyError::runtime_error("expected throw method"));
                                }
                            };
                            let fixed = PyObjectRef::imm(PyObject::BuiltinMethod {
                                name: n,
                                func: f,
                                self_obj: args[0].clone(),
                            });
                            let exc = if args.len() > 1 {
                                args[1].clone()
                            } else {
                                crate::object::py_none()
                            };
                            let mut vm = crate::vm::VirtualMachine::new();
                            vm.call_function(fixed, vec![exc], vec![])
                        } else {
                            Err(PyError::runtime_error("athrow on non-coroutine"))
                        }
                    },
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                "aclose" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: "aclose".to_string(),
                    func: |args| {
                        if let PyObject::Coroutine { frame } = &*args[0].borrow() {
                            let mut frame_opt = frame.borrow_mut();
                            *frame_opt = None;
                            Ok(crate::object::py_none())
                        } else {
                            Err(PyError::runtime_error("aclose on non-coroutine"))
                        }
                    },
                    self_obj: PyObjectRef::new(PyObject::None),
                })),
                _ => Err(PyError::attribute_error(format!(
                    "'coroutine' object has no attribute '{}'",
                    name
                ))),
            },
            PyObject::Process {
                child,
                pid,
                returncode,
                stdin_pipe,
                stdout_pipe,
                stderr_pipe,
            } => {
                match name {
                    "pid" => Ok(py_int(*pid)),
                    "returncode" => Ok(returncode.borrow().map(py_int).unwrap_or_else(py_none)),
                    // `Popen.stdout`/`stdin`/`stderr` — real CPython exposes
                    // the pipe file objects here (test_quopri's cleanup
                    // closes them; test_cmd_line_script's interactive_python
                    // WRITES to stdin and READS the prompt back from the
                    // output pipes). Returns a File wrapping the actual pipe
                    // captured at Popen construction.
                    "stdout" | "stderr" | "stdin" => {
                        let pipe = match name {
                            "stdout" => stdout_pipe.as_ref(),
                            "stderr" => stderr_pipe.as_ref(),
                            _ => stdin_pipe.as_ref(),
                        };
                        if let Some(p) = pipe {
                            Ok(PyObjectRef::new(PyObject::File {
                                file: p.clone(),
                                name: "<pipe>".to_string(),
                                binary: true,
                                pending: std::rc::Rc::new(std::cell::RefCell::new(Vec::new())),
                                closed: false,
                            }))
                        } else if let Ok(f) =
                            std::fs::OpenOptions::new().read(true).open("/dev/null")
                        {
                            Ok(PyObjectRef::new(PyObject::File {
                                file: std::rc::Rc::new(std::cell::RefCell::new(f)),
                                name: "<pipe>".to_string(),
                                binary: true,
                                pending: std::rc::Rc::new(std::cell::RefCell::new(Vec::new())),
                                closed: false,
                            }))
                        } else {
                            Err(PyError::runtime_error(
                                "cannot open /dev/null for Popen pipe",
                            ))
                        }
                    }
                    "poll" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "poll".to_string(),
                        func: |args| {
                            if let PyObject::Process {
                                child, returncode, ..
                            } = &*args[0].borrow()
                            {
                                if let Some(rc) = *returncode.borrow() {
                                    return Ok(py_int(rc));
                                }
                                let mut child_opt = child.borrow_mut();
                                match child_opt.as_mut() {
                                    Some(c) => match c.try_wait() {
                                        Ok(Some(status)) => {
                                            let rc = status.code().unwrap_or(-1) as i64;
                                            *returncode.borrow_mut() = Some(rc);
                                            Ok(py_int(rc))
                                        }
                                        Ok(None) => Ok(py_none()),
                                        Err(e) => Err(PyError::os_error_from_io(&e)),
                                    },
                                    None => Ok(py_none()),
                                }
                            } else {
                                Err(PyError::runtime_error("poll on non-process"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "wait" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "wait".to_string(),
                        func: |args| {
                            if let PyObject::Process {
                                child, returncode, ..
                            } = &*args[0].borrow()
                            {
                                if let Some(rc) = *returncode.borrow() {
                                    return Ok(py_int(rc));
                                }
                                let mut child_opt = child.borrow_mut();
                                match child_opt.as_mut() {
                                    Some(c) => match c.wait() {
                                        Ok(status) => {
                                            let rc = status.code().unwrap_or(-1) as i64;
                                            *returncode.borrow_mut() = Some(rc);
                                            Ok(py_int(rc))
                                        }
                                        Err(e) => Err(PyError::os_error_from_io(&e)),
                                    },
                                    None => Ok(py_none()),
                                }
                            } else {
                                Err(PyError::runtime_error("wait on non-process"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    // `communicate(input=None, timeout=None)` — writes
                    // `input` (if given and stdin was piped) then reads
                    // stdout/stderr to completion via `Child::
                    // wait_with_output` (which internally spawns reader
                    // threads for both streams concurrently, avoiding the
                    // classic "write blocks because the child's stdout
                    // pipe filled up while nobody's reading it yet"
                    // deadlock). Consumes the stored `Child` — a second
                    // `communicate()` call after the first sees `None` and
                    // returns empty output, matching real Python's own
                    // "communicate() should only be called once" contract
                    // closely enough for real-world usage.
                    "communicate" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "communicate".to_string(),
                        func: |args| {
                            // Clone the Process internals out first so we can
                            // mutate the Process (closing stdin) without
                            // holding a borrow on it.
                            let (child, returncode, stdin_pipe, stdout_pipe, stderr_pipe) = {
                                match &*args[0].borrow() {
                                    PyObject::Process {
                                        child,
                                        returncode,
                                        stdin_pipe,
                                        stdout_pipe,
                                        stderr_pipe,
                                        ..
                                    } => (
                                        child.clone(),
                                        returncode.clone(),
                                        stdin_pipe.clone(),
                                        stdout_pipe.clone(),
                                        stderr_pipe.clone(),
                                    ),
                                    _ => {
                                        return Err(PyError::runtime_error(
                                            "communicate on non-process",
                                        ))
                                    }
                                }
                            };
                            let input = args
                                .get(1)
                                .filter(|v| !matches!(&*v.borrow(), PyObject::None));
                            // Write the input to the child's stdin pipe.
                            if let (Some(inp), Some(stdin)) = (input, &stdin_pipe) {
                                use std::io::Write;
                                let bytes = match &*inp.borrow() {
                                    PyObject::Bytes(b) => b.clone(),
                                    other => other.str().into_bytes(),
                                };
                                let _ = stdin.borrow_mut().write_all(&bytes);
                            }
                            // CLOSE stdin so the child sees EOF and can
                            // finish (a child reading stdin blocks until the
                            // write end closes — not closing here deadlocked
                            // communicate() against -mquopri, which reads all
                            // of stdin before producing output). The Process's
                            // slot AND our own cloned handle must both drop.
                            if stdin_pipe.is_some() {
                                if let PyObject::Process { stdin_pipe: sp, .. } =
                                    &mut *args[0].borrow_mut()
                                {
                                    *sp = None;
                                }
                            }
                            drop(stdin_pipe);
                            // Read stdout + stderr pipes to EOF.
                            use std::io::Read;
                            let read_all =
                                |p: &std::rc::Rc<std::cell::RefCell<std::fs::File>>| -> Vec<u8> {
                                    let mut buf = Vec::new();
                                    let _ = p.borrow_mut().read_to_end(&mut buf);
                                    buf
                                };
                            let stdout = stdout_pipe.as_ref().map(read_all).unwrap_or_default();
                            let stderr = stderr_pipe.as_ref().map(read_all).unwrap_or_default();
                            // Reap the child for its returncode.
                            let taken = child.borrow_mut().take();
                            let rc = match taken {
                                Some(mut c) => match c.wait() {
                                    Ok(status) => status.code().unwrap_or(-1) as i64,
                                    Err(_) => -1,
                                },
                                None => returncode.borrow().unwrap_or(-1),
                            };
                            *returncode.borrow_mut() = Some(rc);
                            Ok(py_tuple(vec![
                                PyObjectRef::imm(PyObject::Bytes(stdout)),
                                PyObjectRef::imm(PyObject::Bytes(stderr)),
                            ]))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    // Rust's `std::process::Child` doesn't distinguish a
                    // graceful SIGTERM from a hard SIGKILL the way real
                    // `Popen.terminate()`/`.kill()` do (POSIX-specific) —
                    // both map to `Child::kill()` here, good enough for the
                    // overwhelming majority of real usage (which just wants
                    // "make the child stop").
                    "terminate" | "kill" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: name.to_string(),
                        func: |args| {
                            if let PyObject::Process { child, .. } = &*args[0].borrow() {
                                if let Some(c) = child.borrow_mut().as_mut() {
                                    let _ = c.kill();
                                }
                                Ok(py_none())
                            } else {
                                Err(PyError::runtime_error("terminate/kill on non-process"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "send_signal" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "send_signal".to_string(),
                        func: |args| {
                            // No portable "send arbitrary signal" in std;
                            // treat any signal as a kill request (correct
                            // for the extremely common SIGTERM/SIGKILL
                            // case, not for exotic signal numbers).
                            if let PyObject::Process { child, .. } = &*args[0].borrow() {
                                if let Some(c) = child.borrow_mut().as_mut() {
                                    let _ = c.kill();
                                }
                                Ok(py_none())
                            } else {
                                Err(PyError::runtime_error("send_signal on non-process"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__enter__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__enter__".to_string(),
                        func: |args| Ok(args[0].clone()),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__exit__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__exit__".to_string(),
                        func: |args| {
                            if let PyObject::Process {
                                child, returncode, ..
                            } = &*args[0].borrow()
                            {
                                if returncode.borrow().is_none() {
                                    if let Some(c) = child.borrow_mut().as_mut() {
                                        if let Ok(status) = c.wait() {
                                            *returncode.borrow_mut() =
                                                Some(status.code().unwrap_or(-1) as i64);
                                        }
                                    }
                                }
                            }
                            Ok(py_bool(false))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    _ => Err(PyError::attribute_error(format!(
                        "'Popen' object has no attribute '{}'",
                        name
                    ))),
                }
            }
            PyObject::File { file: f_rc, .. } => return file::get(self, name),
            // `array.array` had NO attributes/methods dispatched at all —
            // even the basics (`.itemsize`, `.typecode`, `.tobytes()`,
            // `.tolist()`) were missing, blocking any real usage beyond
            // construction/indexing. Found via `test_memoryview.py`'s own
            // `BaseArrayMemoryTests`, whose class body reads `array.array
            // ('i').itemsize` — a collection-time crash for the WHOLE file
            // otherwise.
            PyObject::Array(arr) => return array::get(self, name),
            PyObject::MemoryView { .. } => {
                let self_ref = PyObjectRef::new(self.clone());
                if let Some(result) = mv_getprop(&self_ref, name) {
                    return result;
                }
                mv_getattr(name).ok_or_else(|| {
                    PyError::attribute_error(format!(
                        "'memoryview' object has no attribute '{}'",
                        name
                    ))
                })
            }
            PyObject::Socket { inner: _ } => return socket::get(self, name),
            PyObject::Thread(inner_arc) => return thread::get(self, name),
            PyObject::Lock(inner_arc) => {
                let _inner_arc = inner_arc.clone();
                match name {
                    "__enter__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__enter__".to_string(),
                        func: |args| {
                            let obj = args[0].borrow();
                            if let PyObject::Lock(inner_arc) = &*obj {
                                let locked = inner_arc.lock().unwrap();
                                if locked
                                    .lock
                                    .compare_exchange(
                                        false,
                                        true,
                                        std::sync::atomic::Ordering::SeqCst,
                                        std::sync::atomic::Ordering::SeqCst,
                                    )
                                    .is_err()
                                {
                                    // Contended: run deferred threads once (they may
                                    // release), then retry; otherwise report deadlock
                                    // instead of spinning forever.
                                    drop(locked);
                                    crate::modules::coop_threads_drain();
                                    let locked = inner_arc.lock().unwrap();
                                    if locked
                                        .lock
                                        .compare_exchange(
                                            false,
                                            true,
                                            std::sync::atomic::Ordering::SeqCst,
                                            std::sync::atomic::Ordering::SeqCst,
                                        )
                                        .is_err()
                                    {
                                        return Err(PyError::runtime_error(
                                            "lock acquire deadlock in single-threaded interpreter",
                                        ));
                                    }
                                } else {
                                    locked.lock.store(true, std::sync::atomic::Ordering::SeqCst);
                                }
                            }
                            Ok(args[0].clone())
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__exit__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__exit__".to_string(),
                        func: |args| {
                            let obj = args[0].borrow();
                            if let PyObject::Lock(inner_arc) = &*obj {
                                let locked = inner_arc.lock().unwrap();
                                locked
                                    .lock
                                    .store(false, std::sync::atomic::Ordering::SeqCst);
                            }
                            Ok(py_none())
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    // `acquire(blocking=True, timeout=-1)` — the old body
                    // ignored BOTH kwargs entirely and always spun on the
                    // atomic flag forever (`while locked.load() { yield_now() }`
                    // with no exit condition beyond the flag itself
                    // clearing). Since this interpreter runs everything
                    // SYNCHRONOUSLY (no real OS threads backing Python-level
                    // threads), nothing else can ever run concurrently to
                    // release an already-held lock — so re-acquiring a lock
                    // already held by "this" logical flow is a hard,
                    // permanent deadlock unless `blocking=False` or a
                    // `timeout` bounds the wait. Confirmed hanging via
                    // `Lib/test/lock_tests.py`'s `test_state_after_timeout`
                    // (`lock.acquire(); lock.acquire(timeout=0.01)`).
                    "acquire" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "acquire".to_string(),
                        func: |args| {
                            let obj = args[0].borrow();
                            if let PyObject::Lock(inner_arc) = &*obj {
                                let kwargs = args.last().and_then(|a| {
                                    if let PyObject::Dict(d) = &*a.borrow() {
                                        Some((**d).clone())
                                    } else {
                                        None
                                    }
                                });
                                let get_kw = |name: &str| -> Option<PyObjectRef> {
                                    kwargs
                                        .as_ref()
                                        .and_then(|d| d.get(&py_str(name)).ok().flatten())
                                };
                                let is_kwargs_dict =
                                    |v: &PyObjectRef| matches!(&*v.borrow(), PyObject::Dict(_));
                                let blocking = get_kw("blocking")
                                    .or_else(|| args.get(1).filter(|a| !is_kwargs_dict(a)).cloned())
                                    .map(|v| v.truthy())
                                    .unwrap_or(true);
                                let timeout = get_kw("timeout")
                                    .or_else(|| args.get(2).filter(|a| !is_kwargs_dict(a)).cloned())
                                    .and_then(|v| v.as_f64());
                                let locked = inner_arc.lock().unwrap();
                                let try_take = || -> bool {
                                    if locked.lock.load(std::sync::atomic::Ordering::SeqCst) {
                                        false
                                    } else {
                                        locked
                                            .lock
                                            .store(true, std::sync::atomic::Ordering::SeqCst);
                                        true
                                    }
                                };
                                if !blocking {
                                    return Ok(py_bool(try_take()));
                                }
                                if let Some(t) = timeout.filter(|t| *t >= 0.0) {
                                    let deadline = std::time::Instant::now()
                                        + std::time::Duration::from_secs_f64(t);
                                    loop {
                                        if try_take() {
                                            return Ok(py_bool(true));
                                        }
                                        if std::time::Instant::now() >= deadline {
                                            return Ok(py_bool(false));
                                        }
                                        std::thread::yield_now();
                                    }
                                }
                                while !try_take() {
                                    std::thread::yield_now();
                                }
                            }
                            Ok(py_bool(true))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "release" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "release".to_string(),
                        func: |args| {
                            let obj = args[0].borrow();
                            if let PyObject::Lock(inner_arc) = &*obj {
                                let locked = inner_arc.lock().unwrap();
                                locked
                                    .lock
                                    .store(false, std::sync::atomic::Ordering::SeqCst);
                            }
                            Ok(py_none())
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "locked" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "locked".to_string(),
                        func: |args| {
                            let obj = args[0].borrow();
                            if let PyObject::Lock(inner_arc) = &*obj {
                                let locked = inner_arc.lock().unwrap();
                                return Ok(py_bool(
                                    locked.lock.load(std::sync::atomic::Ordering::SeqCst),
                                ));
                            }
                            Ok(py_bool(false))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    _ => Err(PyError::attribute_error(format!(
                        "'lock' object has no attribute '{}'",
                        name
                    ))),
                }
            }
            PyObject::RLock(inner_arc) => {
                let _inner_arc = inner_arc.clone();
                match name {
                    "acquire" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "acquire".to_string(),
                        func: |args| {
                            let obj = args[0].borrow();
                            if let PyObject::RLock(inner_arc) = &*obj {
                                let mut inner = inner_arc.lock().unwrap();
                                let current_id = std::thread::current().id();
                                if let Some(owner) = inner.owner {
                                    if owner == current_id {
                                        inner.count += 1;
                                        return Ok(py_bool(true));
                                    }
                                }
                                // Spin waiting for lock
                                while inner.owner.is_some() {
                                    drop(inner);
                                    std::thread::yield_now();
                                    inner = inner_arc.lock().unwrap();
                                }
                                inner.owner = Some(current_id);
                                inner.count = 1;
                            }
                            Ok(py_bool(true))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "release" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "release".to_string(),
                        func: |args| {
                            let obj = args[0].borrow();
                            if let PyObject::RLock(inner_arc) = &*obj {
                                let mut inner = inner_arc.lock().unwrap();
                                let current_id = std::thread::current().id();
                                if inner.owner != Some(current_id) {
                                    return Err(PyError::runtime_error(
                                        "cannot release un-acquired lock",
                                    ));
                                }
                                inner.count -= 1;
                                if inner.count == 0 {
                                    inner.owner = None;
                                }
                            }
                            Ok(py_none())
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__enter__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__enter__".to_string(),
                        func: |args| {
                            let obj = args[0].borrow();
                            if let PyObject::RLock(inner_arc) = &*obj {
                                let mut inner = inner_arc.lock().unwrap();
                                let current_id = std::thread::current().id();
                                if let Some(owner) = inner.owner {
                                    if owner == current_id {
                                        inner.count += 1;
                                        return Ok(args[0].clone());
                                    }
                                }
                                while inner.owner.is_some() {
                                    drop(inner);
                                    std::thread::yield_now();
                                    inner = inner_arc.lock().unwrap();
                                }
                                inner.owner = Some(current_id);
                                inner.count = 1;
                            }
                            Ok(args[0].clone())
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__exit__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__exit__".to_string(),
                        func: |args| {
                            let obj = args[0].borrow();
                            if let PyObject::RLock(inner_arc) = &*obj {
                                let mut inner = inner_arc.lock().unwrap();
                                let current_id = std::thread::current().id();
                                if inner.owner != Some(current_id) {
                                    return Err(PyError::runtime_error(
                                        "cannot release un-acquired lock",
                                    ));
                                }
                                inner.count -= 1;
                                if inner.count == 0 {
                                    inner.owner = None;
                                }
                            }
                            Ok(py_none())
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    _ => Err(PyError::attribute_error(format!(
                        "'RLock' object has no attribute '{}'",
                        name
                    ))),
                }
            }
            PyObject::Event(inner_arc) => {
                let _inner_arc = inner_arc.clone();
                match name {
                    "is_set" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "is_set".to_string(),
                        func: |args| {
                            let obj = args[0].borrow();
                            if let PyObject::Event(inner_arc) = &*obj {
                                let flag = inner_arc.flag.lock().unwrap();
                                return Ok(py_bool(*flag));
                            }
                            Ok(py_bool(false))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "set" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "set".to_string(),
                        func: |args| {
                            let obj = args[0].borrow();
                            if let PyObject::Event(inner_arc) = &*obj {
                                let mut flag = inner_arc.flag.lock().unwrap();
                                *flag = true;
                                inner_arc.condvar.notify_all();
                            }
                            Ok(py_none())
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "clear" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "clear".to_string(),
                        func: |args| {
                            let obj = args[0].borrow();
                            if let PyObject::Event(inner_arc) = &*obj {
                                let mut flag = inner_arc.flag.lock().unwrap();
                                *flag = false;
                            }
                            Ok(py_none())
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "wait" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "wait".to_string(),
                        func: |args| {
                            let obj = args[0].borrow();
                            if let PyObject::Event(inner_arc) = &*obj {
                                // Cooperative scheduling: first run any deferred
                                // thread bodies (they may set() the event), then
                                // report the flag. If the pending queue is empty
                                // and the event is still unset, NOTHING left in
                                // this single-threaded interpreter can ever set
                                // it -- spinning here would deadlock against the
                                // very continuation that would call set()
                                // (bpo-17141-style), so return the current flag.
                                crate::modules::coop_threads_drain();
                                let flag = inner_arc.flag.lock().unwrap();
                                if !*flag && crate::modules::coop_blocked_forever() {
                                    // Deferred body blocked on an event that
                                    // nothing left can set: unwind this body.
                                    drop(flag);
                                    return Err(PyError::StopIteration);
                                }
                                return Ok(py_bool(*flag));
                            }
                            Ok(py_bool(true))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    _ => Err(PyError::attribute_error(format!(
                        "'Event' object has no attribute '{}'",
                        name
                    ))),
                }
            }
            PyObject::Queue(inner_arc) => {
                let _inner_arc = inner_arc.clone();
                match name {
                    "put" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "put".to_string(),
                        func: |args| {
                            let obj = args[0].borrow();
                            if let PyObject::Queue(inner_arc) = &*obj {
                                let item = args
                                    .get(1)
                                    .cloned()
                                    .ok_or_else(|| PyError::type_error("put() missing argument"))?;
                                let mut q = inner_arc.lock().unwrap();
                                q.queue.push_back(item);
                            }
                            Ok(py_none())
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "get" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "get".to_string(),
                        func: |args| {
                            let obj = args[0].borrow();
                            if let PyObject::Queue(inner_arc) = &*obj {
                                let mut q = inner_arc.lock().unwrap();
                                // Cooperative scheduling: an empty queue may be
                                // waiting on a deferred producer thread.
                                if q.queue.is_empty() {
                                    drop(q);
                                    crate::modules::coop_threads_drain();
                                    q = inner_arc.lock().unwrap();
                                }
                                return q
                                    .queue
                                    .pop_front()
                                    .ok_or_else(|| PyError::runtime_error("empty queue"));
                            }
                            Err(PyError::runtime_error("not a Queue"))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "qsize" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "qsize".to_string(),
                        func: |args| {
                            let obj = args[0].borrow();
                            if let PyObject::Queue(inner_arc) = &*obj {
                                let q = inner_arc.lock().unwrap();
                                return Ok(py_int(q.queue.len() as i64));
                            }
                            Ok(py_int(0))
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    _ => Err(PyError::attribute_error(format!(
                        "'Queue' object has no attribute '{}'",
                        name
                    ))),
                }
            }
            PyObject::Int(_) | PyObject::Bool(_) => return int::get(self, name),
            PyObject::Float(_) => return float::get(self, name),
            PyObject::CompiledRegex { .. } => return compiled_regex::get(self, name),
            PyObject::Super { cls, obj } => {
                // super(cls, obj).attr: walk MRO of obj's type, starting after cls.
                // When `obj` is itself a class/type — the "classmethod-style"
                // form real Python uses for metaclass methods, e.g. inside a
                // metaclass's `def __new__(metacls, name, bases, ns):`, where
                // bare `super()` binds obj=metacls — the relevant mro is
                // `obj`'s own (e.g. a metaclass's own mro), not some further
                // "type of obj" (which would just be `type`/whatever built
                // it, an unrelated chain). Without this, `super().__new__(...)`
                // inside a metaclass's `__new__` couldn't resolve `__new__`
                // at all (AttributeError), since `obj` isn't a plain Instance
                // and has no meaningful `__class__` for this purpose either.
                let obj_types: Vec<PyObjectRef> = if let PyObject::Instance { typ, .. } = &*obj.borrow() {
                    vec![typ.clone()]
                } else if matches!(&*obj.borrow(), PyObject::Type { .. }) {
                    // For a Type obj, try its own MRO first (e.g. super() inside __new__ where obj is metacls)
                    // and if cls not found there, try its metatype's MRO (e.g. super() inside __init__ where obj is a class instance of the metaclass)
                    let mut v = vec![obj.clone()];
                    if let Some(mt) = crate::object::metatype_of(&obj) {
                        v.push(mt);
                    } else if let Ok(cls_attr) = obj.borrow().get_attribute("__class__") {
                        // fallback: type(obj)
                        if !v.iter().any(|x| x.is(&cls_attr)) {
                            v.push(cls_attr);
                        }
                    }
                    v
                } else {
                    match obj.borrow().get_attribute("__class__").ok() {
                        Some(c) => vec![c],
                        None => vec![],
                    }
                };
                for obj_type in obj_types {
                    if let PyObject::Type { mro, .. } = &*obj_type.borrow() {
                        if std::env::var("RPY_DEBUG_SUPER2").is_ok() {
                            let cls_name = cls.borrow().type_name().to_string();
                            let mro_names: Vec<String> =
                                mro.iter().map(|m| m.borrow().type_name()).collect();
                            eprintln!(
                                "SUPER2 cls={} obj_type={} mro={:?} name={}",
                                cls_name,
                                obj_type.borrow().type_name(),
                                mro_names,
                                name
                            );
                        }
                        // Find cls in MRO, start search from the next class.
                        // If `cls` isn't in `obj`'s MRO at all — e.g. a
                        // zero-arg `super()`'s compiled-in `LOAD_GLOBAL
                        // <ClassName>` (see compile_expr's PEP 3135 handling)
                        // picked up a DIFFERENT object than the class this
                        // method actually belongs to, because that global
                        // name got rebound/re-imported to something else in
                        // the meantime — `unwrap_or(0) + 1` used to silently
                        // treat "not found" as "found at position 0", i.e.
                        // start the search at `mro[1]`. For a method whose
                        // own class IS in `obj`'s real MRO (the overwhelmingly
                        // common case, just not reachable via this wrong
                        // `cls`), `mro[1]` is often that SAME class again —
                        // so `super().method()` calls itself again as if it
                        // were the next-in-MRO implementation, forever.
                        // Confirmed via a general, Django-free repro
                        // (rebinding a class's own name inside its
                        // `__init_subclass__` before the trailing
                        // `super().__init_subclass__()` call reproduces
                        // unbounded recursion). Real CPython raises
                        // `TypeError: super(type, obj): obj must be an
                        // instance or subtype of type` here instead — treat
                        // it as "not found via this MRO" and fall through to
                        // the native-backing/error handling below, which is
                        // at least a clean, immediate failure rather than a
                        // silent infinite loop.
                        // Real identity check via `.is()` — the previous
                        // hand-rolled match only ever compared two `Mut`
                        // variants (`Rc::ptr_eq`), silently returning
                        // `false` for anything else. Class/`Type` objects
                        // in this codebase are NOT guaranteed to be `Mut`
                        // (several are `Imm`), so `super(C, e)` — the
                        // EXPLICIT two-argument form, as opposed to the
                        // compiler-synthesized zero-arg one, which happened
                        // to always deal with `Mut` classes in whatever
                        // cases exercised it before — could never find
                        // `cls` in `obj`'s mro at all, making EVERY
                        // attribute lookup through such a `super()` object
                        // fail with `AttributeError`. Confirmed via
                        // CPython's own `test_super.py::test_pickling`
                        // (`s = super(C, e); s.f()`).
                        let start_idx = mro.iter().position(|m| cls.is(m)).map(|p| p + 1);
                        if let Some(start_idx) = start_idx {
                            if start_idx < mro.len() {
                                let mut found = None;
                                for base in mro.iter().skip(start_idx) {
                                    // A builtin exception base (`class MyError
                                    // (OSError): ...`) is a `PyObject::
                                    // BuiltinFunction` (the exception's own
                                    // constructor), never a real `PyObject::
                                    // Type` — invisible to the dict-lookup
                                    // walk just below, so `super().__init__
                                    // (...)` inside such a subclass's own
                                    // `__init__` always raised `AttributeError:
                                    // 'super' object has no attribute
                                    // '__init__'` instead of reaching real
                                    // `BaseException.__init__`'s behavior
                                    // (store the given args as `self.args`).
                                    // Extremely common idiom — any custom
                                    // exception hierarchy that calls
                                    // `super().__init__(...)` (real trigger:
                                    // `urllib.error.URLError(OSError)`).
                                    if name == "__init__" {
                                        if let PyObject::BuiltinFunction { name: bname, .. } =
                                            &*base.borrow()
                                        {
                                            if is_builtin_exception_class_name(bname) {
                                                let target = obj.clone();
                                                found = Some(PyObjectRef::new(PyObject::Closure(Rc::new(move |args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                                                if let PyObject::Instance { dict, .. } = &mut *target.borrow_mut() {
                                                    dict.insert("args".to_string(), py_tuple(args.to_vec()));
                                                }
                                                Ok(py_none())
                                            }))));
                                                break;
                                            }
                                        }
                                    }
                                    if let PyObject::Type { dict, .. } = &*base.borrow() {
                                        if let Some(val) = dict.get_str(&name) {
                                            let val_borrowed = val.borrow();
                                            match &*val_borrowed {
                                                // `__new__` is *always* implicitly
                                                // a staticmethod in real Python —
                                                // never auto-bound — regardless of
                                                // whether it's explicitly wrapped
                                                // in `staticmethod(...)`. Only the
                                                // explicit-wrapper case was
                                                // unwrapped below; a plain `def
                                                // __new__(mcs, ...):` (which is
                                                // how virtually every real
                                                // metaclass, including Django's,
                                                // writes it — nobody bothers with
                                                // `@staticmethod` there) still hit
                                                // the auto-bind arm just below,
                                                // producing a BoundMethod that
                                                // prepended `obj` as an EXTRA,
                                                // duplicate leading argument on
                                                // top of the one already passed
                                                // explicitly (`super().__new__(mcs,
                                                // name, bases, attrs)` always
                                                // passes `mcs` itself) — shifting
                                                // every subsequent positional arg
                                                // by one.
                                                PyObject::Function(_)
                                                | PyObject::BuiltinFunction { .. }
                                                    if name == "__new__" =>
                                                {
                                                    found = Some(val.clone());
                                                    break;
                                                }
                                                PyObject::Function(_)
                                                | PyObject::BuiltinFunction { .. } => {
                                                    found = Some(PyObjectRef::new(
                                                        PyObject::BoundMethod {
                                                            func: val.clone(),
                                                            self_obj: obj.clone(),
                                                        },
                                                    ));
                                                    break;
                                                }
                                                // A method found directly in a
                                                // migrated native type's own
                                                // dict (e.g. `dict.__setitem__`,
                                                // `dict.__getitem__`) is stored
                                                // as a `BuiltinMethod` with a
                                                // PLACEHOLDER `self_obj` (see
                                                // `NATIVE_VALUE_CTOR_KEY`'s doc
                                                // comment) — the catch-all arm
                                                // below returns such values
                                                // UNCHANGED, which is correct
                                                // for genuine bound instance
                                                // methods (their `self_obj` is
                                                // already the right target) but
                                                // WRONG here: this placeholder
                                                // must be rebound to `obj` (the
                                                // real instance `super()` was
                                                // constructed for), exactly like
                                                // the `Function`/`BuiltinFunction`
                                                // case just above. Missing this
                                                // meant `super().__setitem__(k, v)`
                                                // inside e.g. `enum.py`'s
                                                // `_EnumDict.__setitem__`
                                                // resolved to `dict.__setitem__`
                                                // with its self_obj STILL the
                                                // placeholder, so the call ended
                                                // up as `dict.__setitem__(None,
                                                // k, v)` instead of `(obj, k,
                                                // v)` — an instant panic
                                                // (`borrow_mut` on the
                                                // placeholder `PyObjectRef::None`,
                                                // which isn't `Mut`).
                                                PyObject::BuiltinMethod {
                                                    name: m_name,
                                                    func,
                                                    ..
                                                } => {
                                                    found = Some(PyObjectRef::imm(
                                                        PyObject::BuiltinMethod {
                                                            name: m_name.clone(),
                                                            func: *func,
                                                            self_obj: obj.clone(),
                                                        },
                                                    ));
                                                    break;
                                                }
                                                PyObject::Property(ref d) if d.getter.is_some() => {
                                                    let g = d.getter.as_ref().unwrap();
                                                    found = Some(
                                                        builtin_call(g, &[obj.clone()])
                                                            .unwrap_or_else(|_| val.clone()),
                                                    );
                                                    break;
                                                }
                                                // Staticmethods (explicit, or
                                                // implicit like `__new__`) are
                                                // never bound to `obj` — unwrap
                                                // directly, matching how plain
                                                // class-attribute access already
                                                // treats StaticMethod.
                                                PyObject::StaticMethod { func } => {
                                                    found = Some(func.clone());
                                                    break;
                                                }
                                                // A `@classmethod`-wrapped method found on an
                                                // ancestor's dict via `super()` (e.g.
                                                // `super().setUpClass()` inside a subclass's own
                                                // `setUpClass` override, real trigger: `unittest`'s
                                                // own `TestCase.setUpClass`/`tearDownClass`) — the
                                                // catch-all arm below returned the raw
                                                // `PyObject::ClassMethod` wrapper UNCHANGED, which
                                                // isn't itself callable (`TypeError: 'classmethod'
                                                // object is not callable`). `obj` here is already
                                                // the class itself in this calling convention (see
                                                // this match's own comment on `obj_type` above, for
                                                // the "obj is a class/type" classmethod-style
                                                // form), so binding is the same shape as the
                                                // `Function`/`BuiltinFunction` case: wrap in a
                                                // `BoundMethod` with `self_obj: obj.clone()`.
                                                PyObject::ClassMethod { func } => {
                                                    found = Some(PyObjectRef::new(
                                                        PyObject::BoundMethod {
                                                            func: func.clone(),
                                                            self_obj: obj.clone(),
                                                        },
                                                    ));
                                                    break;
                                                }
                                                _ => {
                                                    found = Some(val.clone());
                                                    break;
                                                }
                                            }
                                        }
                                    }
                                }
                                if let Some(found) = found {
                                    return Ok(found);
                                }
                            }
                        }
                    }
                }
                // Not found via any Type in the mro: for a class that
                // transparently subclasses list/dict/str, `super().append(x)`
                // etc. must still reach the native backing (list/dict/str
                // themselves aren't PyObject::Type, so they're invisible to
                // the mro walk above).
                if name == "__init__" {
                    if let Some(kind) = native_base_of_type(&{
                        if let PyObject::Instance { typ, .. } = &*obj.borrow() {
                            typ.clone()
                        } else {
                            return Err(PyError::attribute_error(
                                "'super' object has no attribute '__init__'".to_string(),
                            ));
                        }
                    }) {
                        let target = obj.clone();
                        return Ok(PyObjectRef::new(PyObject::Closure(Rc::new(
                            move |args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                                let native = synthesize_native_init(&kind, args, &[])?;
                                if let PyObject::Instance { dict, .. } = &mut *target.borrow_mut() {
                                    dict.insert(NATIVE_BACKING_KEY.to_string(), native);
                                }
                                Ok(py_none())
                            },
                        ))));
                    }
                }
                // `super().__setattr__(name, value)`/`__delattr__(name)` —
                // the real `object.__setattr__`/`__delattr__` (a plain
                // generic attribute set/delete) isn't exposed as a gettable
                // attribute anywhere either (same class of gap as
                // `__init__` just above), needed by real code that
                // deliberately bypasses an overridden `__setattr__` this
                // way (a frozen-dataclass-style pattern — real trigger:
                // CPython 3.14's own `Lib/_colorize.py`'s
                // `ThemeSection.__post_init__`).
                if name == "__setattr__" || name == "__delattr__" {
                    let target = obj.clone();
                    let is_delete = name == "__delattr__";
                    return Ok(PyObjectRef::new(PyObject::Closure(Rc::new(
                        move |args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                            if args.is_empty() {
                                return Err(PyError::type_error("missing required argument: name"));
                            }
                            let attr_name = args[0].str();
                            if is_delete {
                                target.borrow_mut().del_attribute(&attr_name)?;
                            } else {
                                if args.len() < 2 {
                                    return Err(PyError::type_error(
                                        "__setattr__() takes exactly 2 arguments",
                                    ));
                                }
                                target
                                    .borrow_mut()
                                    .set_attribute(&attr_name, args[1].clone())?;
                            }
                            Ok(py_none())
                        },
                    ))));
                }
                // Same story for the operator-level dunders — list/dict
                // don't expose __setitem__/__getitem__/etc. as a plain
                // get_attribute entry either (subscripting/len/iteration go
                // through their own opcode-level dispatch functions
                // instead), so synthesize a callable that invokes those
                // functions directly against the real native backing.
                if let Some(native) = native_backing_of(obj) {
                    let target = native.clone();
                    match name {
                        "__setitem__" => {
                            return Ok(PyObjectRef::new(PyObject::Closure(Rc::new(
                                move |args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                                    if args.len() < 2 {
                                        return Err(PyError::type_error(
                                            "__setitem__() takes exactly 2 arguments",
                                        ));
                                    }
                                    py_setitem(&target, &args[0], args[1].clone())?;
                                    Ok(py_none())
                                },
                            ))));
                        }
                        "__getitem__" => {
                            return Ok(PyObjectRef::new(PyObject::Closure(Rc::new(
                                move |args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                                    if args.is_empty() {
                                        return Err(PyError::type_error(
                                            "__getitem__() takes exactly 1 argument",
                                        ));
                                    }
                                    py_getitem(&target, &args[0])
                                },
                            ))));
                        }
                        "__delitem__" => {
                            return Ok(PyObjectRef::new(PyObject::Closure(Rc::new(
                                move |args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                                    if args.is_empty() {
                                        return Err(PyError::type_error(
                                            "__delitem__() takes exactly 1 argument",
                                        ));
                                    }
                                    py_delitem(&target, &args[0])?;
                                    Ok(py_none())
                                },
                            ))));
                        }
                        "__contains__" => {
                            return Ok(PyObjectRef::new(PyObject::Closure(Rc::new(
                                move |args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                                    if args.is_empty() {
                                        return Err(PyError::type_error(
                                            "__contains__() takes exactly 1 argument",
                                        ));
                                    }
                                    Ok(py_bool(contains_op(&target, &args[0])?))
                                },
                            ))));
                        }
                        "__len__" => {
                            return Ok(PyObjectRef::new(PyObject::Closure(Rc::new(
                                move |_args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                                    builtin_len(&[target.clone()])
                                },
                            ))));
                        }
                        "__iter__" => {
                            return Ok(PyObjectRef::new(PyObject::Closure(Rc::new(
                                move |_args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                                    builtin_iter(&[target.clone()])
                                },
                            ))));
                        }
                        _ => {}
                    }
                }
                if let Some(native) = native_backing_of(obj) {
                    if let Ok(val) = native.borrow().get_attribute(&name) {
                        let rebound =
                            if let PyObject::BuiltinMethod { name: n, func, .. } = &*val.borrow() {
                                PyObjectRef::imm(PyObject::BuiltinMethod {
                                    name: n.clone(),
                                    func: *func,
                                    self_obj: native.clone(),
                                })
                            } else {
                                val.clone()
                            };
                        return Ok(rebound);
                    }
                }
                Err(PyError::attribute_error(format!(
                    "'super' object has no attribute '{}'",
                    name
                )))
            }
            PyObject::FutureAwaitIterator {
                future: _,
                yielded: _,
            } => {
                match name {
                    "__iter__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__iter__".to_string(),
                        func: |args| Ok(args[0].clone()),
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__next__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__next__".to_string(),
                        func: |args| {
                            if args.is_empty() {
                                return Err(PyError::type_error("__next__ needs self"));
                            }
                            let self_ref = args[0].borrow();
                            let (done, result) = match &*self_ref {
                                PyObject::FutureAwaitIterator { future, yielded } => {
                                    if *yielded {
                                        let done = future
                                            .borrow()
                                            .get_attribute("_done")
                                            .ok()
                                            .map(|d| d.truthy())
                                            .unwrap_or(false);
                                        let result = future
                                            .borrow()
                                            .get_attribute("_result")
                                            .unwrap_or_else(|_| py_none());
                                        (Some(done), Some(result))
                                    } else {
                                        let f = future.clone();
                                        drop(self_ref);
                                        return Ok(f);
                                    }
                                }
                                _ => {
                                    return Err(PyError::runtime_error(
                                        "__next__ on non-FutureAwaitIterator",
                                    ))
                                }
                            };
                            drop(self_ref);
                            if let Some(true) = done {
                                Err(PyError::Exception(
                                    "StopIteration".to_string(),
                                    result.unwrap_or_else(|| py_none()),
                                ))
                            } else {
                                Ok(py_none())
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "send" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "send".to_string(),
                        func: |args| {
                            if args.is_empty() {
                                return Err(PyError::type_error("send needs self"));
                            }
                            let (is_first, future_clone) = match &*args[0].borrow() {
                                PyObject::FutureAwaitIterator { future, yielded } => {
                                    (!*yielded, future.clone())
                                }
                                _ => {
                                    return Err(PyError::runtime_error(
                                        "send on non-FutureAwaitIterator",
                                    ))
                                }
                            };
                            if is_first {
                                let mut obj = args[0].borrow_mut();
                                if let PyObject::FutureAwaitIterator { yielded, .. } = &mut *obj {
                                    *yielded = true;
                                }
                                drop(obj);
                                // Return the future as the yielded value
                                Ok(future_clone)
                            } else {
                                // Second send: check if future is done
                                let done = future_clone
                                    .borrow()
                                    .get_attribute("_done")
                                    .ok()
                                    .map(|d| d.truthy())
                                    .unwrap_or(false);
                                let result = future_clone
                                    .borrow()
                                    .get_attribute("_result")
                                    .unwrap_or_else(|_| py_none());
                                if done {
                                    Err(PyError::Exception("StopIteration".to_string(), result))
                                } else {
                                    Ok(future_clone)
                                }
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    _ => Err(PyError::attribute_error(format!(
                        "'future_await_iterator' object has no attribute '{}'",
                        name
                    ))),
                }
            }
            PyObject::BuiltinFunction {
                name: bf_name,
                func,
            } => {
                if bf_name == "memoryview" {
                    if name == "_from_flags" {
                        return Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                            name: "_from_flags".to_string(),
                            func: crate::object::mv_from_flags,
                            self_obj: PyObjectRef::new(PyObject::None),
                        }));
                    }
                }
                if bf_name == "bytes" && name == "fromhex" {
                    return Ok(PyObjectRef::imm(PyObject::BuiltinFunction {
                        name: "fromhex".to_string(),
                        func: builtin_bytes_fromhex,
                    }));
                }
                if bf_name == "complex" && name == "from_number" {
                    return Ok(PyObjectRef::imm(PyObject::BuiltinFunction {
                        name: "from_number".to_string(),
                        func: |args| {
                            if args.is_empty() {
                                return Err(PyError::type_error(
                                    "complex.from_number() takes exactly 1 argument",
                                ));
                            }
                            let n = args[0].as_f64().unwrap_or(0.0);
                            Ok(PyObjectRef::imm(PyObject::Complex(n, 0.0)))
                        },
                    }));
                }
                if bf_name == "float" && name == "__getformat__" {
                    // `float.__getformat__("double"/"float")` — real CPython
                    // queries the platform's actual float representation;
                    // this interpreter's floats are always IEEE 754 doubles
                    // (Rust `f64`), so always answer accordingly. Real
                    // trigger: CPython's own `test.support.requires_IEEE_754`
                    // module-level constant, `float.__getformat__("double").
                    // startswith("IEEE")`.
                    return Ok(PyObjectRef::imm(PyObject::BuiltinFunction {
                        name: "__getformat__".to_string(),
                        func: |_args| Ok(py_str("IEEE, little-endian")),
                    }));
                }
                if bf_name == "float" && name == "fromhex" {
                    return Ok(PyObjectRef::imm(PyObject::BuiltinFunction {
                        name: "fromhex".to_string(),
                        func: |args| {
                            if args.is_empty() {
                                return Err(PyError::type_error(
                                    "float.fromhex() requires exactly 1 argument",
                                ));
                            }
                            let s = args[0].str();
                            let s = s.trim();
                            let lower = s.to_lowercase();
                            if lower == "nan" {
                                return Ok(py_float(f64::NAN));
                            }
                            if lower == "inf"
                                || lower == "+inf"
                                || lower == "-inf"
                                || lower == "infinity"
                                || lower == "+infinity"
                                || lower == "-infinity"
                            {
                                let sign = if lower.starts_with('-') { -1.0 } else { 1.0 };
                                return Ok(py_float(sign * f64::INFINITY));
                            }
                            let s = s.strip_prefix("+").unwrap_or(s);
                            let sign = if s.starts_with('-') { -1.0 } else { 1.0 };
                            let s = s
                                .strip_prefix('-')
                                .unwrap_or(s.strip_prefix('+').unwrap_or(s));
                            let s = s
                                .strip_prefix("0x")
                                .or_else(|| s.strip_prefix("0X"))
                                .ok_or_else(|| {
                                    PyError::value_error(format!(
                                        "invalid hex float literal: {}",
                                        s
                                    ))
                                })?;
                            let (int_part, rest) = s.split_once('.').unwrap_or((s, ""));
                            let (frac_part, exp_part) = rest
                                .split_once('p')
                                .or_else(|| rest.split_once('P'))
                                .unwrap_or((rest, ""));
                            let int_val = i64::from_str_radix(int_part, 16).unwrap_or(0);
                            let frac_val = if !frac_part.is_empty() {
                                let frac_bits = i64::from_str_radix(frac_part, 16).unwrap_or(0);
                                let frac_len = frac_part.len() as u32;
                                frac_bits as f64 / (16u64.pow(frac_len) as f64)
                            } else {
                                0.0
                            };
                            let exp: i32 = if !exp_part.is_empty() {
                                exp_part.parse().map_err(|_| {
                                    PyError::value_error(format!(
                                        "invalid hex float exponent: {}",
                                        exp_part
                                    ))
                                })?
                            } else {
                                0
                            };
                            let significand = int_val as f64 + frac_val;
                            let result = sign * significand * (2.0f64).powi(exp);
                            Ok(py_float(result))
                        },
                    }));
                }
                if bf_name == "float" && name == "hex" {
                    return Ok(PyObjectRef::imm(PyObject::BuiltinFunction {
                        name: "hex".to_string(),
                        func: |args| {
                            if args.is_empty() {
                                return Err(PyError::type_error("hex() takes exactly 1 argument"));
                            }
                            let obj = args[0].borrow();
                            if let PyObject::Float(v) = &*obj {
                                let bits = v.to_bits();
                                let sign = if (bits >> 63) != 0 { "-" } else { "" };
                                let biased_exp = ((bits >> 52) & 0x7ff) as i64;
                                let mantissa = bits & 0x000f_ffff_ffff_ffff;
                                if biased_exp == 0x7ff {
                                    if mantissa == 0 {
                                        Ok(py_str(&format!("{}inf", sign)))
                                    } else {
                                        Ok(py_str(&format!("{}nan", sign)))
                                    }
                                } else if *v == 0.0 {
                                    Ok(py_str(&format!("{}0x0.0p+0", sign)))
                                } else {
                                    let exp = biased_exp - 1023;
                                    let hex_mantissa = format!("{:013x}", mantissa);
                                    let hex_mantissa = hex_mantissa.trim_end_matches('0');
                                    Ok(py_str(&format!(
                                        "{}0x1.{}p{:+}",
                                        sign,
                                        if hex_mantissa.is_empty() {
                                            "0"
                                        } else {
                                            hex_mantissa
                                        },
                                        exp
                                    )))
                                }
                            } else {
                                Err(PyError::type_error("hex() argument must be float"))
                            }
                        },
                    }));
                }
                if bf_name == "float" && name == "from_number" {
                    return Ok(PyObjectRef::imm(PyObject::BuiltinFunction {
                        name: "from_number".to_string(),
                        func: |args| {
                            if args.is_empty() {
                                return Err(PyError::type_error(
                                    "float.from_number() takes exactly 1 argument",
                                ));
                            }
                            Ok(py_float(args[0].as_f64().unwrap_or(f64::NAN)))
                        },
                    }));
                }
                if bf_name == "int" && name == "from_bytes" {
                    return Ok(PyObjectRef::imm(PyObject::BuiltinFunction {
                        name: "from_bytes".to_string(),
                        func: builtin_int_from_bytes,
                    }));
                }
                if bf_name == "dict" && name == "fromkeys" {
                    // dict.fromkeys(iterable, value=None) — a real classmethod
                    // in CPython, called both as `dict.fromkeys(...)` and via
                    // `cls.fromkeys(...)` inside a dict-subclass's own
                    // methods (real code: `collections.ChainMap.__iter__`
                    // does `dict.fromkeys(mapping)`). Missing entirely before
                    // — `dict` has no attribute dict of its own to answer
                    // this from, being a plain BuiltinFunction constructor
                    // rather than a real Type.
                    return Ok(PyObjectRef::imm(PyObject::BuiltinFunction {
                        name: "fromkeys".to_string(),
                        func: |args| {
                            if args.is_empty() {
                                return Err(PyError::type_error(
                                    "fromkeys() takes at least 1 argument",
                                ));
                            }
                            let keys = crate::object::collect_iterable(&args[0])?;
                            let value = args.get(1).cloned().unwrap_or_else(py_none);
                            let mut d = PyDict::new();
                            for k in keys {
                                d.set(k, value.clone())?;
                            }
                            Ok(PyObjectRef::new(PyObject::Dict(Box::new(d))))
                        },
                    }));
                }
                if bf_name == "dict" && (name == "__setitem__" || name == "__getitem__") {
                    let method_name = name.to_string();
                    return Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: method_name.clone(),
                        func: if method_name == "__setitem__" {
                            builtin_dict_setitem as BuiltinFunc
                        } else {
                            builtin_dict_getitem as BuiltinFunc
                        },
                        self_obj: py_none(),
                    }));
                }
                // Built-in types (int, str, list, dict, ...) are represented
                // as a plain callable BuiltinFunction here, not a real class
                // object with its own bases/mro — so `int.mro()`-style
                // introspection (used e.g. by Django's lazy() for wrapping
                // arbitrary result types) has nothing real to walk. Returning
                // just [self] is not a correct ancestor chain (misses
                // `object`, and any real base for exception types etc.), but
                // it lets that code iterate something instead of crashing.
                if name == "mro" {
                    return Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "mro".to_string(),
                        func: |args| Ok(py_list(vec![args[0].clone()])),
                        self_obj: py_none(),
                    }));
                }
                if name == "__name__" {
                    return Ok(py_str(bf_name));
                }
                if name == "__qualname__" {
                    return Ok(py_str(bf_name));
                }
                // Same gap, same fix, as the real `PyObject::Type`'s own
                // `__module__` fallback just above — this is the OTHER
                // ad-hoc-type representation (built-in exception "classes"),
                // which need it too (e.g. `Exception.__module__`).
                if name == "__module__" {
                    return Ok(py_str("builtins"));
                }
                if name == "__mro__" || name == "__bases__" {
                    return Ok(PyObjectRef::new(PyObject::Tuple(vec![])));
                }
                if name == "__dict__" {
                    return Ok(PyObjectRef::new(PyObject::Dict(Box::new(PyDict::new()))));
                }
                if bf_name == "bool" && name == "__new__" {
                    return Ok(PyObjectRef::imm(PyObject::BuiltinFunction {
                        name: "__new__".to_string(),
                        func: |args| {
                            if args.is_empty() {
                                return Ok(py_bool(false));
                            }
                            if args.len() >= 2 {
                                return Ok(py_bool(args[1].truthy()));
                            }
                            Ok(py_bool(false))
                        },
                    }));
                }
                // A handful of generic dunders every real builtin function/
                // type has in CPython, regardless of which specific one —
                // were missing across the board (not one-off gaps), so
                // adding them here (rather than per-name like `fromhex`/
                // `__getformat__` above) covers `int`/`str`/`list`/`dict`/
                // any other native constructor uniformly. Real trigger:
                // CPython's own `test_heapq.py` (`__module__`), `test_call.py`/
                // `test_structseq.py` (`__new__`/`__init__` — common
                // "is this constructible via type.__new__" introspection),
                // `test_complex.py` (`__hash__` — checking hashability).
                if name == "__module__" {
                    return Ok(py_str("builtins"));
                }
                if name == "__hash__" {
                    return Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__hash__".to_string(),
                        func: |args| Ok(py_int(args[0].hash()? as i64)),
                        self_obj: py_none(),
                    }));
                }
                if name == "__new__" || name == "__init__" {
                    // CPython: `int.__new__(bool, ...)` raises TypeError
                    // ("int.__new__(bool) is not safe, use bool.__new__()")
                    // — bool has its own allocator. test_bool::test_subclass.
                    if name == "__new__" && bf_name == "int" {
                        return Ok(PyObjectRef::imm(PyObject::BuiltinFunction {
                            name: "int.__new__".to_string(),
                            func: int_new_checked,
                        }));
                    }
                    // Pragmatic stand-in: real CPython's builtin `__new__`/
                    // `__init__` slots are the actual C-level allocators/
                    // initializers, not separately-callable Python-visible
                    // functions with independent behavior worth
                    // reimplementing here — returning the constructor
                    // itself is "good enough" for introspection code that
                    // just checks these exist/are callable (real trigger:
                    // `test_structseq.py`'s `SomeStructType.__new__`-based
                    // construction pattern) without claiming to model the
                    // real two-phase alloc/init protocol.
                    return Ok(PyObjectRef::imm(PyObject::BuiltinFunction {
                        name: bf_name.clone(),
                        func: *func,
                    }));
                }
                Err(PyError::attribute_error(format!(
                    "'{}' object has no attribute '{}'",
                    self.type_name(),
                    name
                )))
            }
            PyObject::FrozenSet(_items) => return frozenset::get(self, name),
            PyObject::Slice { .. } => return slice::get(self, name),
            PyObject::Code(c) => {
                match name {
                    "co_filename" => Ok(py_str(crate::interner::lookup_str(c.filename))),
                    "co_name" => Ok(py_str(crate::interner::lookup_str(c.name))),
                    "co_argcount" => Ok(py_int(c.arg_count as i64)),
                    "co_nlocals" => Ok(py_int(c.nlocals as i64)),
                    "co_varnames" => Ok(py_tuple(
                        c.varnames
                            .iter()
                            .map(|&v| py_str(crate::interner::lookup_str(v)))
                            .collect(),
                    )),
                    "co_flags" => Ok(py_int(c.flags as i64)),
                    // A handful of other commonly-introspected `co_*`
                    // fields were missing entirely (`AttributeError`) —
                    // real trigger: CPython's own `test_super.py`'s direct
                    // `func.__code__.co_firstlineno` check, among others.
                    "co_firstlineno" => Ok(py_int(c.first_lineno as i64)),
                    "co_kwonlyargcount" => Ok(py_int(c.kwonlyarg_count as i64)),
                    "co_posonlyargcount" => Ok(py_int(c.posonlyarg_count as i64)),
                    "co_names" => Ok(py_tuple(
                        c.names
                            .iter()
                            .map(|&v| py_str(crate::interner::lookup_str(v)))
                            .collect(),
                    )),
                    "co_consts" => Ok(py_tuple(
                        c.consts
                            .iter()
                            .filter_map(|cv| crate::vm::eval_const_value(cv.clone()).ok())
                            .collect(),
                    )),
                    _ => Err(PyError::attribute_error(format!(
                        "'code' object has no attribute '{}'",
                        name
                    ))),
                }
            }
            PyObject::Range { .. } | PyObject::RangeIter { .. } => return range::get(self, name),
            PyObject::BuiltinMethod { name: bm_name, func, self_obj } => match name {
                "__self__" => Ok(self_obj.clone()),
                "__func__" => Ok(PyObjectRef::new(PyObject::BuiltinFunction {
                    name: bm_name.clone(),
                    func: *func,
                })),
                "__name__" => Ok(py_str(bm_name)),
                "__qualname__" => Ok(py_str(bm_name)),
                "__module__" => Ok(py_str("builtins")),
                "__doc__" => Ok(py_none()),
                _ => Err(PyError::attribute_error(format!(
                    "'builtin_function_or_method' object has no attribute '{}'",
                    name
                ))),
            },
            PyObject::ListIter { .. }
            | PyObject::MapIterator { .. }
            | PyObject::FilterIterator { .. }
            | PyObject::ZipIterator { .. }
            | PyObject::CycleIter { .. }
            | PyObject::GroupByIter { .. }
            | PyObject::EnumerateIter { .. }
            | PyObject::GetItemIter { .. }
            | PyObject::CallSentinelIter { .. }
                if name == "__next__" || name == "__iter__" =>
            {
                Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: name.to_string(),
                    func: if name == "__next__" {
                        builtin_next
                    } else {
                        builtin_iter
                    },
                    self_obj: PyObjectRef::new(self.clone()),
                }))
            }
            _ => Err(PyError::attribute_error(format!(
                "'{}' object has no attribute '{}'",
                self.type_name(),
                name
            ))),
        }
    }
}

impl ObjectAccess for PyObject {
    fn get_attribute(&self, name: &str) -> PyResult<PyObjectRef> {
        match self.get_attribute_impl(name) {
            Err(_) if name == "__doc__" => Ok(py_none()),
            Err(e) if matches!(e, PyError::AttributeError(_)) => {
                // Attach the attribute NAME and owning OBJECT to the raised
                // AttributeError (CPython: `exc.name`/`exc.obj`). This is
                // what `except AttributeError as exc: exc.name` sees after
                // `obj.missing_attr`. The reconstructed PyObjectRef for
                // `self` differs in identity from the original Rc, so
                // `exc.obj is obj` may be False, but value equality and the
                // overwhelmingly common `exc.name` checks work.
                let mut extra = std::collections::HashMap::new();
                extra.insert("name".to_string(), py_str(name));
                extra.insert("obj".to_string(), PyObjectRef::new(self.clone()));
                Err(PyError::Exception(
                    "AttributeError".to_string(),
                    PyObjectRef::new(PyObject::Exception {
                        typ: "AttributeError".to_string(),
                        args: vec![py_str(&format!(
                            "'{}' object has no attribute '{}'",
                            self.type_name(),
                            name
                        ))],
                        cause: None,
                        suppress_context: false,
                        context: None,
                        traceback: None,
                        extra: Some(extra),
                    }),
                ))
            }
            other => other,
        }
    }

    fn set_attribute(&mut self, name: &str, value: PyObjectRef) -> PyResult<()> {
        match self {
            PyObject::Instance { dict, typ } => {
                if crate::object::get_type_name_for_instance(typ) == "Dialect" {
                    return Err(PyError::AttributeError("attribute is read-only".to_string()));
                }
                // Check __slots__ restriction if defined on the type or its MRO
                if let Some(slots) = get_instance_slots(typ) {
                    if !slots.iter().any(|s| s == name) {
                        let type_name = get_type_name_for_instance(typ);
                        return Err(PyError::attribute_error(format!(
                            "'{}' object has no attribute '{}'",
                            type_name, name
                        )));
                    }
                }
                dict.insert_str(&name, value);
                Ok(())
            }
            PyObject::Module { dict, name: mod_name } => {
                dict.insert_str(&name, value.clone());
                // Keep `frame.globals` (the Rc captured by functions
                // defined in this module) in sync with `module.__dict__`
                // when `setattr(module, name, value)` is used (e.g.
                // `mock.patch.object(script_helper, 'interpreter_requires_environment',
                // return_value=True)`). `LOAD_GLOBAL` inside
                // `run_python_until_end` reads from `frame.globals`, not
                // `module.dict`, so without this the mock is invisible.
                crate::object::pydict::update_module_globals(mod_name, name, value.clone());
                Ok(())
            }
            PyObject::Type { dict, .. } => {
                dict.insert_str(&name, value);
                Ok(())
            }
            PyObject::Function(ref mut f) => {
                f.dict.insert_str(&name, value);
                Ok(())
            }
            PyObject::Dict(_)
            | PyObject::List(_)
            | PyObject::Tuple(_)
            | PyObject::Set(_)
            | PyObject::FrozenSet(_) => {
                // Store attributes in a side dict (instance-like) for these built-in types
                let _pd = match self {
                    PyObject::Dict(d) => Some(d.clone()),
                    _ => None,
                };
                Err(PyError::attribute_error(format!(
                    "cannot set attribute '{}' on '{}'",
                    name,
                    self.type_name()
                )))
            }
            PyObject::Exception {
                cause,
                suppress_context,
                ..
            } if name == "__cause__" => {
                *cause = Some(value);
                Ok(())
            }
            PyObject::Exception { context, .. } if name == "__context__" => {
                *context = Some(value);
                Ok(())
            }
            PyObject::Exception { traceback, .. } if name == "__traceback__" => {
                *traceback = Some(value);
                Ok(())
            }
            PyObject::Exception {
                suppress_context, ..
            } if name == "__suppress_context__" => {
                let b = value.borrow();
                *suppress_context = matches!(&*b, PyObject::Bool(true));
                Ok(())
            }
            PyObject::Exception { extra, .. } => {
                // Store arbitrary per-instance attributes (BaseException
                // `__dict__` semantics): `e.name = ...`, `e.obj = ...`, etc.
                // This also backs the AttributeError name/obj set by the
                // getattr machinery. `__traceback__`/`__context__` etc. are
                // handled by dedicated arms above.
                let extra = extra.get_or_insert_with(|| std::collections::HashMap::new());
                extra.insert(name.to_string(), value);
                Ok(())
            }
            PyObject::Partial { dict, .. } => {
                dict.insert_str(name, value);
                Ok(())
            }
            PyObject::ExceptionGroup { .. } => {
                // No backing dict on these variants for __traceback__,
                // __context__, __suppress_context__, __notes__, or custom
                // attributes — but `except E as e: e.__traceback__ = tb` (and
                // similar) is an extremely common idiom (contextlib,
                // unittest, ...) that must not hard-crash just because we
                // don't track those fields anywhere.
                Ok(())
            }
            PyObject::WeakProxy { target, .. } => {
                if let Some(rc) = target.upgrade() {
                    let t = PyObjectRef::Imm(rc);
                    return t.borrow_mut().set_attribute(name, value);
                } else {
                    return Err(PyError::reference_error("weakly-referenced object no longer exists"));
                }
            }
            _ => Err(PyError::attribute_error(format!(
                "cannot set attribute '{}' on '{}'",
                name,
                self.type_name()
            ))),
        }
    }

    fn del_attribute(&mut self, name: &str) -> PyResult<()> {
        match self {
            PyObject::Instance { dict, typ } => {
                if crate::object::get_type_name_for_instance(typ) == "Dialect" {
                    return Err(PyError::AttributeError("attribute is read-only".to_string()));
                }
                // Check __slots__ restriction if defined on the type or its MRO
                if let Some(slots) = get_instance_slots(typ) {
                    if !slots.iter().any(|s| s == name) {
                        let type_name = get_type_name_for_instance(typ);
                        return Err(PyError::attribute_error(format!(
                            "'{}' object has no attribute '{}'",
                            type_name, name
                        )));
                    }
                }
                dict.remove(name).ok_or_else(|| {
                    PyError::attribute_error(format!(
                        "'{}' object has no attribute '{}'",
                        self.type_name(),
                        name
                    ))
                })?;
                Ok(())
            }
            PyObject::Module { dict, .. } => {
                dict.remove(&interner::intern(name)).ok_or_else(|| {
                    PyError::attribute_error(format!("module has no attribute '{}'", name))
                })?;
                Ok(())
            }
            PyObject::Type { dict, .. } => {
                dict.remove(&interner::intern(name)).ok_or_else(|| {
                    PyError::attribute_error(format!("type has no attribute '{}'", name))
                })?;
                Ok(())
            }
            PyObject::WeakProxy { target, .. } => {
                if let Some(rc) = target.upgrade() {
                    let t = PyObjectRef::Imm(rc);
                    return t.borrow_mut().del_attribute(name);
                } else {
                    return Err(PyError::reference_error("weakly-referenced object no longer exists"));
                }
            }
            _ => Err(PyError::attribute_error(format!(
                "'{}' object has no attribute '{}'",
                self.type_name(),
                name
            ))),
        }
    }
}
