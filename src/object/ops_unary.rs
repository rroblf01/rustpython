// Split out of the former monolithic object.rs (see the file-splitting
// refactor's memory entry for context) — this file holds unary numeric
// operations (`-`, `not`) plus the shared `hash_bigint`/`as_complex_parts`
// helpers used across the numeric-tower operations.
use super::*;

// ---- CPython str/bytes hash: DJBX33A (short) + SipHash13 (long) ----

/// The per-process hash secret derived from PYTHONHASHSEED, mirroring
/// CPython's `_Py_HashSecret`: `(siphash_k0, siphash_k1, djbx33a_suffix)`.
/// Seed 0 (or "0") disables randomization; a numeric seed fills the 24-byte
/// secret with CPython's `lcg_urandom`; "random"/unset uses process entropy.
fn hash_secret() -> (u64, u64, u64) {
    use std::sync::OnceLock;
    static SECRET: OnceLock<(u64, u64, u64)> = OnceLock::new();
    *SECRET.get_or_init(|| {
        let seed_text = std::env::var("PYTHONHASHSEED").unwrap_or_default();
        let seed: u64 = if seed_text == "random" || seed_text.is_empty() {
            let t = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0);
            t ^ (std::process::id() as u64)
        } else {
            seed_text.parse().unwrap_or(0)
        };
        if seed == 0 {
            (0, 0, 0)
        } else {
            // CPython's lcg_urandom(seed, secret, 24).
            let mut x = seed as u32;
            let mut out = [0u8; 24];
            for b in out.iter_mut() {
                x = x.wrapping_mul(214013).wrapping_add(2531011);
                *b = ((x >> 16) & 0xff) as u8;
            }
            let k0 = u64::from_le_bytes(out[0..8].try_into().unwrap());
            let k1 = u64::from_le_bytes(out[8..16].try_into().unwrap());
            let suffix = u64::from_le_bytes(out[16..24].try_into().unwrap());
            (k0, k1, suffix)
        }
    })
}

fn sip_half_round(a: u64, b: u64, c: u64, d: u64, s: u32, t: u32) -> (u64, u64, u64, u64) {
    let a = a.wrapping_add(b);
    let c = c.wrapping_add(d);
    let b = b.rotate_left(s) ^ a;
    let d = d.rotate_left(t) ^ c;
    (a.rotate_left(32), b, c, d)
}

fn sip_single_round(v0: u64, v1: u64, v2: u64, v3: u64) -> (u64, u64, u64, u64) {
    let (v0, v1, v2, v3) = sip_half_round(v0, v1, v2, v3, 13, 16);
    // HALF_ROUND(v2, v1, v0, v3, 17, 21): a=v2, b=v1, c=v0, d=v3.
    let (a, b, c, d) = sip_half_round(v2, v1, v0, v3, 17, 21);
    (c, b, a, d) // a=new_v2, b=new_v1, c=new_v0, d=new_v3
}

/// CPython's SipHash-1-3 (Python/pyhash.c), returning the raw "modified"
/// finalization value `(v0 ^ v1) ^ (v2 ^ v3)`.
fn siphash13(k0: u64, k1: u64, data: &[u8]) -> u64 {
    let (mut v0, mut v1, mut v2, mut v3) = (
        k0 ^ 0x736f6d6570736575,
        k1 ^ 0x646f72616e646f6d,
        k0 ^ 0x6c7967656e657261,
        k1 ^ 0x7465646279746573,
    );
    let mut b: u64 = (data.len() as u64) << 56;
    let mut chunks = data.chunks_exact(8);
    for chunk in &mut chunks {
        let mi = u64::from_le_bytes(chunk.try_into().unwrap());
        v3 ^= mi;
        (v0, v1, v2, v3) = sip_single_round(v0, v1, v2, v3);
        v0 ^= mi;
    }
    let mut t = 0u64;
    for (i, &c) in chunks.remainder().iter().enumerate() {
        t |= (c as u64) << (8 * i);
    }
    b |= t;
    v3 ^= b;
    (v0, v1, v2, v3) = sip_single_round(v0, v1, v2, v3);
    v0 ^= b;
    v2 ^= 0xff;
    (v0, v1, v2, v3) = sip_single_round(v0, v1, v2, v3);
    (v0, v1, v2, v3) = sip_single_round(v0, v1, v2, v3);
    (v0, v1, v2, v3) = sip_single_round(v0, v1, v2, v3);
    (v0 ^ v1) ^ (v2 ^ v3)
}

/// CPython's `_Py_HashBytes` — SipHash13 for everything (Py_HASH_CUTOFF is 0
/// in modern CPython, so the DJBX33A short-string path is compiled out);
/// -1 remapped to -2.
pub(crate) fn py_hash_bytes(data: &[u8]) -> usize {
    if data.is_empty() {
        return 0; // CPython's _Py_HashBytes returns 0 for len 0
    }
    let (k0, k1, _suffix) = hash_secret();
    let x = siphash13(k0, k1, data) as i64;
    (if x == -1 { -2 } else { x }) as usize
}

/// CPython's str hash: the string hashed over its NARROWEST code-point
/// representation (UCS1/2/4, little-endian per char) — not UTF-8 bytes for
/// non-ASCII (test_hash's test_ucs2_string).
pub(crate) fn py_hash_str(s: &str) -> usize {
    let max = s.chars().map(|c| c as u32).max().unwrap_or(0);
    let mut v: Vec<u8> = Vec::with_capacity(s.len());
    if max < 0x100 {
        for c in s.chars() {
            v.push((c as u32 & 0xff) as u8);
        }
    } else if max < 0x10000 {
        for c in s.chars() {
            let cp = c as u32;
            v.push((cp & 0xff) as u8);
            v.push((cp >> 8) as u8);
        }
    } else {
        for c in s.chars() {
            v.extend_from_slice(&(c as u32).to_le_bytes());
        }
    }
    py_hash_bytes(&v)
}

/// The int hash — CPython's `long_hash`: lower bits for anything that fits in
/// a machine word (the overwhelming common case for dict/set keys and for the
/// `hash()` builtin), and for larger magnitudes CPython's real
/// mod-(2**61-1) rotation over 30-bit digits (matching `hash_double`, so
/// `hash(float(sys.float_info.max)) == hash(int(sys.float_info.max))`
/// holds — a value a byte-XOR scan would get wrong). `-1` is remapped to
/// `-2`, matching CPython's own special case (C-level code reserves -1 as an
/// internal "hash computation failed" sentinel, so a real hash that happens
/// to compute to -1 is bumped to -2 instead).
pub(crate) fn hash_bigint(i: &BigInt) -> usize {
    // CPython's single-digit fast path: |n| < 2**30 hashes to itself (with
    // the -1 -> -2 sentinel remap). Anything larger goes through the modular
    // rotation below — a naive `return n` for all machine words would
    // disagree with CPython for n in [2**30, 2**61) (e.g. hash(2**61-1) is
    // 0, not 2**61-1) and hence with `hash_double` of the equal float.
    if let Some(n) = i.to_i64() {
        if n > -(1i64 << 30) && n < (1i64 << 30) {
            return if n == -1 {
                (-2i64) as usize
            } else {
                n as usize
            };
        }
    }
    const SHIFT: u32 = 30; // CPython PyLong_SHIFT
    const BITS: u32 = 61; // CPython _PyHASH_BITS
    const MOD: u64 = (1u64 << BITS) - 1; // CPython _PyHASH_MODULUS
    let neg = i.sign() == num_bigint::Sign::Minus;
    let mut t = i.abs();
    // Extract base-2**30 digits, least significant first.
    let mask = BigInt::from((1u64 << SHIFT) - 1);
    let mut digits: Vec<u32> = Vec::new();
    while t != BigInt::zero() {
        digits.push((&t & &mask).to_u64().unwrap_or(0) as u32);
        t >>= SHIFT;
    }
    // CPython processes the MOST significant digit first.
    let mut x: u64 = 0;
    for &d in digits.iter().rev() {
        x = ((x << SHIFT) & MOD) | (x >> (BITS - SHIFT));
        x += d as u64;
        if x >= MOD {
            x -= MOD;
        }
    }
    let mut result = if neg { 0u64.wrapping_sub(x) } else { x };
    if result == u64::MAX {
        result = u64::MAX - 1; // -1 -> -2
    }
    result as usize
}

/// CPython's `_Py_HashDouble` for a finite/non-NaN `f64`: the value is
/// treated as the exact rational `m * 2**e` and hashed mod 2**61-1 via a
/// 28-bit-at-a-time rotation. NaN is NOT handled here (it hashes by object
/// identity in CPython, so callers decide how to represent it); infinities
/// hash to `±_PyHASH_INF` (314159).
pub(crate) fn hash_double(v: f64) -> usize {
    if v.is_infinite() {
        return if v > 0.0 {
            314159usize
        } else {
            (-314159i64) as usize
        };
    }
    if v == 0.0 {
        return 0;
    }
    const BITS: u32 = 61; // CPython _PyHASH_BITS
    const MOD: u64 = (1u64 << BITS) - 1; // CPython _PyHASH_MODULUS
                                         // frexp: v = m * 2**e with 0.5 <= |m| < 1, computed from the IEEE bits
                                         // (a `2f64.powi(e)` scale would overflow to inf for e near ±1024).
    let (m0, e0) = {
        let bits = v.abs().to_bits();
        let biased = ((bits >> 52) & 0x7ff) as i64;
        let mantissa = bits & 0x000f_ffff_ffff_ffff;
        if biased == 0 {
            // subnormal: v = mantissa * 2**-1074; shift so the top bit sits
            // at 2**-1 (0.5 <= m < 1). mantissa is non-zero here (v != 0).
            let t = 63 - mantissa.leading_zeros(); // bit index of the MSB
            let m = mantissa as f64 / (1u64 << (t + 1)) as f64;
            (m, t as i32 + 1 - 1074)
        } else {
            // normal: v = 1.mantissa * 2**(biased-1023).
            let full = (1u64 << 52) | mantissa;
            let m = full as f64 / (1u64 << 53) as f64; // in [0.5, 1)
            (m, (biased - 1022) as i32)
        }
    };
    let mut m = m0; // m0 is already |v|'s mantissa (positive)
    let sign = if v < 0.0 { -1i64 } else { 1i64 };
    let mut e = e0;
    let mut x: u64 = 0;
    while m != 0.0 {
        // Rotate the 61-bit accumulator left by 28 and add the next 28-bit
        // digit (CPython processes 28 bits at a time).
        x = ((x << 28) & MOD) | (x >> (BITS - 28));
        m *= 268435456.0; // 2**28
        e -= 28;
        let y = m as u64; // pull out the integer part (m < 2**28)
        m -= y as f64;
        x += y;
        if x >= MOD {
            x -= MOD;
        }
    }
    // Adjust for the exponent: rotate by (e mod 61).
    e = if e >= 0 {
        e % BITS as i32
    } else {
        BITS as i32 - 1 - ((-1 - e) % BITS as i32)
    };
    x = ((x << (e as u32)) & MOD) | (x >> (BITS - e as u32));
    let mut result = if sign < 0 { 0u64.wrapping_sub(x) } else { x };
    if result == u64::MAX {
        result = u64::MAX - 1; // -1 -> -2
    }
    result as usize
}

/// Extracts (real, imaginary) from any of `complex`/`int`/`float`/`bool` —
/// used to let complex arithmetic transparently accept a real-number operand
/// on either side (`1 + 2j`, `2j * 3.0`, ...) without a combinatorial
/// explosion of match arms per numeric-type pairing.
pub(crate) fn as_complex_parts(obj: &PyObject) -> Option<(f64, f64)> {
    match obj {
        PyObject::Complex(re, im) => Some((*re, *im)),
        PyObject::Int(n) => n.to_f64().map(|f| (f, 0.0)),
        PyObject::Float(f) => Some((*f, 0.0)),
        PyObject::Bool(b) => Some((if *b { 1.0 } else { 0.0 }, 0.0)),
        PyObject::Instance { .. } => {
            // `Fraction` participates in the numeric tower for complex
            // arithmetic (`Fraction(1,2) + 1j` -> 0.5+1j in real CPython).
            let cloned = PyObjectRef::new(obj.clone());
            if let Some((n, d)) = crate::modules::frac_instance_num_den(&cloned) {
                return Some((crate::modules::frac_to_f64(&n, &d), 0.0));
            }
            None
        }
        _ => None,
    }
}

pub fn py_neg(val: &PyObjectRef) -> PyResult<PyObjectRef> {
    if let Some(i) = val.as_i64() {
        // -i64::MIN overflows; it is exactly 2**63, a BigInt.
        return match i.checked_neg() {
            Some(n) => Ok(py_int(n)),
            None => Ok(py_int(-BigInt::from(i))),
        };
    }
    // A boxed BigInt must negate EXACTLY (not via as_f64, which loses
    // precision) — my UNARY_NEGATIVE rework routes big-int negation here.
    let b = val.borrow();
    match &*b {
        PyObject::Int(n) => Ok(py_int(-n.clone())),
        PyObject::Float(n) => Ok(py_float(-n)),
        PyObject::Complex(re, im) => Ok(PyObjectRef::imm(PyObject::Complex(-re, -im))),
        PyObject::Instance { .. } => {
            // Native-backing subclasses negate their underlying value
            // (-IntSubclass(5) is -5, an instance).
            let typ = match &*b {
                PyObject::Instance { typ, .. } => typ.clone(),
                _ => unreachable!(),
            };
            if let Some(native) = crate::object::native_backing_of(val) {
                let negated = py_neg(&native)?;
                return Ok(crate::object::make_subclass_instance(&typ, negated));
            }
            Err(PyError::type_error(format!(
                "bad operand type for unary -: '{}'",
                b.type_name()
            )))
        }
        _ => {
            drop(b);
            if let Some(f) = val.as_f64() {
                return Ok(py_float(-f));
            }
            Err(PyError::type_error(format!(
                "bad operand type for unary -: '{}'",
                val.borrow().type_name()
            )))
        }
    }
}

pub fn py_not(val: &PyObjectRef) -> PyResult<PyObjectRef> {
    Ok(py_bool(!val.try_truthy()?))
}

pub fn py_pos(val: &PyObjectRef) -> PyResult<PyObjectRef> {
    // `+bool` yields the int 0/1 (a NEW int object, not the same bool) —
    // test_bool's test_math asserts `+False is not False`.
    if let PyObjectRef::SmallBool(b) = val {
        return Ok(py_int(if *b { 1 } else { 0 }));
    }
    if val.as_i64().is_some() || val.as_f64().is_some() {
        return Ok(val.clone());
    }
    let obj = val.borrow();
    match &*obj {
        PyObject::Int(_) | PyObject::Float(_) | PyObject::Complex(_, _) => {
            drop(obj);
            Ok(val.clone())
        }
        _ => Err(PyError::type_error(format!(
            "bad operand type for unary +: '{}'",
            obj.type_name()
        ))),
    }
}
