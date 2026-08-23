//! Native `_random` module — MT19937 in Rust, faithful to CPython's
//! `_randommodule.c`. `Lib/random.py` subclasses `_random.Random` exactly
//! like real CPython does; the previous pure-Python generator (MWC64X with
//! O(n^2) big-int shifts in `getrandbits`) made `getrandbits(2**31)` take
//! effectively forever and stalled the whole test sweep.
//!
//! Per-instance state (624xu32 + index) lives in the instance dict as one
//! 2504-byte `Bytes` blob so the native methods round-trip it without
//! converting 624 Python ints on every call.

use crate::object::*;
use num_bigint::BigInt;
use std::collections::HashMap;

const N: usize = 624;
const M: usize = 397;
const MATRIX_A: u32 = 0x9908_b0df;
const UPPER_MASK: u32 = 0x8000_0000;
const LOWER_MASK: u32 = 0x7fff_ffff;

#[derive(Clone)]
struct Mt {
    mt: [u32; N],
    idx: usize,
}

impl Mt {
    fn new() -> Self {
        let mut mt = [0u32; N];
        mt[0] = 1_965_021_8;
        for i in 1..N {
            mt[i] = 1_812_433_253u32
                .wrapping_mul(mt[i - 1] ^ (mt[i - 1] >> 30))
                .wrapping_add(i as u32);
        }
        Mt { mt, idx: N }
    }

    fn init_by_array(&mut self, key: &[u32]) {
        let mut i = 1usize;
        let mut j = 0usize;
        let mut k = N.max(key.len());
        while k > 0 {
            let prev = if i >= 1 { self.mt[i - 1] } else { self.mt[N - 1] };
            self.mt[i] = (self.mt[i]
                ^ (prev ^ (prev >> 30)).wrapping_mul(1_664_525))
            .wrapping_add(key.get(j).copied().unwrap_or(0))
            .wrapping_add(j as u32);
            i += 1;
            j += 1;
            if i >= N {
                self.mt[0] = self.mt[N - 1];
                i = 1;
            }
            if j >= key.len() {
                j = 0;
            }
            k -= 1;
        }
        k = N - 1;
        while k > 0 {
            let prev = if i >= 1 { self.mt[i - 1] } else { self.mt[N - 1] };
            self.mt[i] = (self.mt[i]
                ^ (prev ^ (prev >> 30)).wrapping_mul(1_566_083_941))
            .wrapping_sub(i as u32);
            i += 1;
            if i >= N {
                self.mt[0] = self.mt[N - 1];
                i = 1;
            }
            k -= 1;
        }
        self.mt[0] = 0x8000_0000;
    }

    fn regenerate(&mut self) {
        for i in 0..N {
            let y = (self.mt[i] & UPPER_MASK) | (self.mt[(i + 1) % N] & LOWER_MASK);
            self.mt[i] = self.mt[(i + M) % N] ^ (y >> 1);
            if y & 1 != 0 {
                self.mt[i] ^= MATRIX_A;
            }
        }
        self.idx = 0;
    }

    fn next_u32(&mut self) -> u32 {
        if self.idx >= N {
            self.regenerate();
        }
        let mut y = self.mt[self.idx];
        self.idx += 1;
        y ^= y >> 11;
        y ^= (y << 7) & 0x9d2c_5680;
        y ^= (y << 15) & 0xefc6_0000;
        y ^= y >> 18;
        y
    }

    fn to_bytes(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(N * 4 + 8);
        for w in &self.mt {
            v.extend_from_slice(&w.to_le_bytes());
        }
        v.extend_from_slice(&(self.idx as u64).to_le_bytes());
        v
    }

    fn from_bytes(b: &[u8]) -> Option<Self> {
        if b.len() < N * 4 + 8 {
            return None;
        }
        let mut mt = [0u32; N];
        for (i, w) in mt.iter_mut().enumerate() {
            let off = i * 4;
            *w = u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]]);
        }
        let idx = u64::from_le_bytes([
            b[N*4], b[N*4+1], b[N*4+2], b[N*4+3],
            b[N*4+4], b[N*4+5], b[N*4+6], b[N*4+7],
        ]) as usize;
        Some(Mt { mt, idx })
    }
}

// ── instance-state helpers ───────────────────────────────────────────

fn get_mt(self_obj: &PyObjectRef) -> PyResult<Mt> {
    let b = self_obj.borrow();
    if let PyObject::Instance { dict, .. } = &*b {
        if let Some(st) = dict.get_str("_mt_state") {
            if let PyObject::Bytes(bytes) = &*st.borrow() {
                return Mt::from_bytes(bytes)
                    .ok_or_else(|| PyError::runtime_error("corrupt _random state"));
            }
        }
    }
    Err(PyError::runtime_error("_random.Random state missing"))
}

fn put_mt(self_obj: &PyObjectRef, mt: Mt) {
    let mut b = self_obj.borrow_mut();
    if let PyObject::Instance { dict, .. } = &mut *b {
        dict.insert_str(
            "_mt_state",
            PyObjectRef::imm(PyObject::Bytes(mt.to_bytes())),
        );
    }
}

/// int → little-endian u32 words of its absolute value (CPython's
/// _randommodule.c seed key derivation).
fn int_to_key(a: &PyObjectRef) -> PyResult<Vec<u32>> {
    let n_big = a
        .as_i64()
        .map(|v| num_bigint::BigInt::from(v))
        .or_else(|| {
            let b = a.borrow();
            if let PyObject::Int(i) = &*b {
                Some(i.clone())
            } else {
                None
            }
        })
        .ok_or_else(|| PyError::type_error("seed must be an int"))?;
    use num_traits::Signed;
    let abs = n_big.abs();
    let (_, bytes) = abs.to_bytes_le();
    let mut words = Vec::new();
    for chunk in bytes.chunks(4) {
        let mut w = [0u8; 4];
        w[..chunk.len()].copy_from_slice(chunk);
        words.push(u32::from_le_bytes(w));
    }
    if words.is_empty() {
        words.push(0);
    }
    Ok(words)
}

// ── module-level constructor + methods ───────────────────────────────

fn make_random_type() -> PyObjectRef {
    let mut type_dict: HashMap<String, PyObjectRef> = HashMap::new();

    macro_rules! rm {
        ($name:expr, $func:expr) => {
            type_dict.insert(
                $name.to_string(),
                PyObjectRef::imm(PyObject::BuiltinFunction {
                    name: $name.to_string(),
                    func: $func,
                }),
            );
        };
    }

    rm!("seed", |args| {
        let self_obj = args
            .first()
            .cloned()
            .ok_or_else(|| PyError::type_error("seed(self) missing"))?;
        let seed_arg = args.get(1).cloned().unwrap_or_else(py_none);
        let mut mt = Mt::new();
        if !matches!(&*seed_arg.borrow(), PyObject::None) {
            mt.init_by_array(&int_to_key(&seed_arg)?);
        }
        put_mt(&self_obj, mt);
        Ok(py_none())
    });

    rm!("getrandbits", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("getrandbits() missing k"));
        }
        let k = args[1]
            .as_i64()
            .ok_or_else(|| PyError::type_error("k must be int"))?;
        if k < 0 {
            return Err(PyError::value_error(
                "number of bits must be greater than zero",
            ));
        }
        if k == 0 {
            return Ok(py_int(0i64));
        }
        let mut mt = get_mt(&args[0])?;
        let words = (((k as u64) + 31) / 32) as usize;
        let last_bits = (k % 32) as u32;
        // Build the result as raw little-endian 32-bit words and convert
        // ONCE at the end — per-word `BigInt` OR/shift is O(n^2) and made
        // getrandbits(2**31) take effectively forever.
        let mut buf = vec![0u8; words * 4];
        for i in 0..words {
            let mut w = mt.next_u32();
            if i + 1 == words && last_bits > 0 {
                w >>= 32 - last_bits;
            }
            buf[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
        }
        put_mt(&args[0], mt);
        // Trim trailing zero bytes so from_bytes_le never sees an empty
        // sign issue and stays cheap.
        while let Some(&0) = buf.last() {
            buf.pop();
        }
        if buf.is_empty() {
            return Ok(py_int(0i64));
        }
        use num_traits::Signed;
        Ok(py_int(BigInt::from_bytes_le(num_bigint::Sign::Plus, &buf)))
    });

    rm!("random", |args| {
        let mut mt = get_mt(&args.first().cloned().unwrap())?;
        let a = (mt.next_u32() >> 5) as f64;
        let b = (mt.next_u32() >> 6) as f64;
        put_mt(&args[0].clone(), mt);
        Ok(py_float((a * 67_108_864.0 + b) * (1.0 / 9_007_199_254_740_992.0)))
    });

    rm!("getstate", |args| {
        let mt = get_mt(&args.first().cloned().unwrap())?;
        let items: Vec<PyObjectRef> = mt.mt.iter().map(|w| py_int(*w)).collect();
        Ok(py_tuple({
            let mut v = items;
            v.push(py_int(mt.idx));
            v.push(py_none());
            v
        }))
    });

    rm!("setstate", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("setstate() missing state"));
        }
        let st = args[1].borrow();
        let tuple = match &*st {
            PyObject::Tuple(t) => t,
            _ => return Err(PyError::type_error("state must be a tuple")),
        };
        if tuple.len() < N + 1 {
            return Err(PyError::value_error("state vector too short"));
        }
        let mut mt = Mt { mt: [0; N], idx: N };
        for (i, item) in tuple.iter().take(N).enumerate() {
            mt.mt[i] = item.as_i64().unwrap_or(0) as u32;
        }
        mt.idx = tuple[N].as_i64().unwrap_or(N as i64).max(0) as usize;
        drop(st);
        put_mt(&args[0].clone(), mt);
        Ok(py_none())
    });

    rm!("__init__", |args| {
        let self_obj = args
            .first()
            .cloned()
            .ok_or_else(|| PyError::type_error("__init__ missing self"))?;
        let seed_arg = args.get(1).cloned().unwrap_or_else(py_none);
        let mut mt = Mt::new();
        if !matches!(&*seed_arg.borrow(), PyObject::None) {
            mt.init_by_array(&int_to_key(&seed_arg)?);
        }
        put_mt(&self_obj, mt);
        Ok(py_none())
    });

    PyObjectRef::new(PyObject::Type {
        name: "Random".to_string(),
        dict: Box::new(crate::object::str_map_to_typedict(type_dict)),
        bases: vec![],
        mro: vec![],
    })
}

/// Builds the native `_random` module dict.
pub fn create_random_dict() -> HashMap<String, PyObjectRef> {
    let mut d: HashMap<String, PyObjectRef> = HashMap::new();
    d.insert("_Random".to_string(), make_random_type());
    d.insert(
        "Random".to_string(),
        d.get("_Random").cloned().expect("just inserted"),
    );
    d.insert(
        "_test_first_genrand".to_string(),
        PyObjectRef::imm(PyObject::BuiltinFunction {
            name: "_test_first_genrand".to_string(),
            func: |args| {
                let mut mt = Mt::new();
                if let Some(a) = args.get(1) {
                    mt.init_by_array(&int_to_key(a)?);
                }
                Ok(py_int(mt.next_u32()))
            },
        }),
    );
    d
}

