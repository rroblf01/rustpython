use crate::object::*;
use num_traits::{Signed, ToPrimitive};

/// Integer value of a `math` integer argument: a native int, an int-subclass
/// instance (its int backing), or any `__index__` object.
pub fn math_int_value(v: &PyObjectRef) -> PyResult<num_bigint::BigInt> {
    if let Some(n) = crate::object::int_value_or_backing(v) {
        return Ok(n);
    }
    crate::object::to_index(v)
}

/// `math` float argument (native float/int, or any `__float__` object —
/// descriptors resolved so a raising `__float__` descriptor propagates).
pub fn math_float_value(v: &PyObjectRef) -> PyResult<f64> {
    if let Some(f) = v.as_f64() {
        return Ok(f);
    }
    let typ = if let PyObject::Instance { typ, .. } = &*v.borrow() {
        typ.clone()
    } else {
        return Err(PyError::type_error("argument must be a number"));
    };
    let Some(f) = lookup_dunder_via_mro(&typ, "__float__") else {
        return Err(PyError::type_error("argument must be a number"));
    };
    let has_get = f.borrow().get_attribute("__get__").is_ok();
    if has_get {
        // Descriptor protocol: `f.__get__(instance, type)`.
        let get = f.borrow().get_attribute("__get__").unwrap();
        let resolved = call_bound_method(get, f.clone(), vec![v.clone(), typ.clone()])?;
        let inner = resolved
            .as_f64()
            .ok_or_else(|| PyError::type_error("__float__ returned non-float"))?;
        return Ok(inner);
    }
    let result = call_bound_method(f, v.clone(), vec![])?;
    result
        .as_f64()
        .ok_or_else(|| PyError::type_error("__float__ returned non-float"))
}

pub fn math_arg_f64(v: &PyObjectRef) -> Option<f64> {
    if let Some(f) = v.as_f64() {
        return Some(f);
    }
    // Float (and int) subclasses like MyFloat(float) store the real
    // numeric value as a native backing (e.g. MyFloat(NaN)). `as_f64()`
    // returns None for Instances, so check the backing directly.
    if let Some(backing) = crate::object::native_backing_of(v) {
        if let Some(f) = backing.as_f64() {
            return Some(f);
        }
    }
    let f = {
        let typ = if let PyObject::Instance { typ, .. } = &*v.borrow() {
            Some(typ.clone())
        } else {
            None
        }?;
        lookup_dunder_via_mro(&typ, "__float__")?
    };
    call_bound_method(f, v.clone(), vec![]).ok()?.as_f64()
}

/// `math.floor`/`math.ceil`/`math.trunc` dispatch to `__floor__`/`__ceil__`/
/// `__trunc__` on an instance when present (a Python `Function` method, a
/// native `BuiltinFunction`/`BuiltinMethod`, or `None` = explicitly
/// disabled, which must raise TypeError rather than fall through). Returns
/// `Ok(None)` when no usable dunder exists.
pub fn math_call_int_dunder(self_obj: &PyObjectRef, name: &str) -> PyResult<Option<PyObjectRef>> {
    let typ = if let PyObject::Instance { typ, .. } = &*self_obj.borrow() {
        typ.clone()
    } else {
        return Ok(None);
    };
    let Some(f) = lookup_dunder_via_mro(&typ, name) else {
        return Ok(None);
    };
    // Generic descriptor protocol (mirrors a real instance attribute
    // access): if the found dunder value is a descriptor — an arbitrary
    // object with `__get__`, e.g. `test_math`'s `BadDescr` — invoke
    // `__get__(obj, type)` and dispatch on its result. CPython propagates
    // whatever `__get__` raises (BadDescr raises `ValueError`) instead of
    // trying to call the raw, non-callable value. Plain functions/methods
    // also carry `__get__`; invoking it here returns the same bound
    // callable the arms below would have dispatched on anyway.
    let f = {
        let get_result = f.borrow().get_attribute("__get__");
        match get_result {
            Ok(get_fn) => crate::object::call_function_disposable(
                &get_fn,
                vec![self_obj.clone(), typ.clone()],
                vec![],
            )?,
            Err(_) => f,
        }
    };
    let b = f.borrow();
    match &*b {
        PyObject::None => Err(PyError::type_error(format!(
            "'{}' object does not support {}",
            crate::object::get_type_name_for_instance(&typ),
            name
        ))),
        PyObject::BuiltinFunction { func, .. } => {
            let func = *func;
            drop(b);
            Ok(Some(func(&[self_obj.clone()])?))
        }
        PyObject::BuiltinMethod { func, .. } => {
            let func = *func;
            drop(b);
            Ok(Some(func(&[self_obj.clone()])?))
        }
        PyObject::BoundMethod { .. } => {
            drop(b);
            Ok(Some(call_bound_method(f, self_obj.clone(), vec![])?))
        }
        _ => {
            drop(b);
            Ok(Some(call_bound_method(f, self_obj.clone(), vec![])?))
        }
    }
}

/// Exact summation of `items` — the "lsum" algorithm behind CPython's
/// `math.fsum` and `math.sumprod`'s float path: each float is split with
/// frexp into (mantissa, exponent), mantissas are aligned and added as big
/// integers, and the exact result is rounded once at the end, so
/// cancellation can never destroy small terms. Handles NaN / ±inf /
/// overflow / underflow the way CPython's fsum does.
pub fn exact_fsum(items: &[f64]) -> PyResult<f64> {
    let mant_dig = 53i64;
    let mut tmant = num_bigint::BigInt::from(0);
    let mut texp: i64 = 0;
    let mut seen = false;
    let mut pos_inf = false;
    let mut neg_inf = false;
    for &x in items {
        if x.is_nan() {
            return Ok(f64::NAN);
        }
        if x.is_infinite() {
            if x > 0.0 {
                pos_inf = true;
            } else {
                neg_inf = true;
            }
            continue;
        }
        if x == 0.0 {
            continue;
        }
        // frexp: x = mant * 2^exp with mant in [0.5, 1)
        let bits = x.to_bits();
        let sign = if bits >> 63 == 0 { 1.0 } else { -1.0 };
        let biased = ((bits >> 52) & 0x7ff) as i64;
        let (mant_mag, exp) = if biased == 0 {
            (f64::from_bits(bits & 0x000f_ffff_ffff_ffff), -1022)
        } else {
            (
                f64::from_bits((bits & 0x000f_ffff_ffff_ffff) | 0x3fe0_0000_0000_0000),
                biased - 1022,
            )
        };
        let mant_i = num_bigint::BigInt::from(crate::object::ldexp_f64(
            sign * mant_mag,
            mant_dig as i32,
        ) as i64);
        let exp = exp - mant_dig;
        if !seen {
            tmant = mant_i;
            texp = exp;
            seen = true;
        } else if texp > exp {
            tmant <<= (texp - exp) as usize;
            texp = exp;
            tmant += mant_i;
        } else {
            tmant += mant_i << ((exp - texp) as usize);
        }
    }
    if pos_inf && neg_inf {
        return Err(PyError::value_error("math domain error"));
    }
    if pos_inf {
        return Ok(f64::INFINITY);
    }
    if neg_inf {
        return Ok(f64::NEG_INFINITY);
    }
    if !seen {
        return Ok(0.0);
    }
    // Round the exact integer result once to a double (round-half-to-even,
    // trimming to 53 significant bits), then scale by the exact power of
    // two 2^texp.
    let neg = tmant.sign() == num_bigint::Sign::Minus;
    let mag = tmant.abs();
    let bits = mag.bits() as i64;
    let etiny = -1074i64; // sys.float_info.min_exp - mant_dig
    let tail = (bits - mant_dig).max(etiny - texp);
    let mut texp_final = texp;
    let m = if tail > 0 {
        let h = num_bigint::BigInt::from(1u64) << ((tail - 1) as usize);
        let two_h = &h << 1;
        let q = &mag / &two_h;
        let half = (&mag & &h) != num_bigint::BigInt::from(0);
        let three_h_minus_1 = &(&(&h << 1) + &h) - 1u32;
        let sticky = (&mag & &three_h_minus_1) != num_bigint::BigInt::from(0);
        texp_final += tail;
        q + (if half && sticky { 1u32 } else { 0u32 })
    } else {
        mag
    };
    let m = if neg { -m } else { m };
    let result = m.to_f64().unwrap_or(0.0);
    let scaled = crate::object::ldexp_f64(result, texp_final.clamp(-2000, 2000) as i32);
    if scaled.is_infinite() {
        return Err(PyError::overflow_error("math range error"));
    }
    Ok(scaled)
}

// Integer -> (m, e) frexp split for logarithms: `n = m * 2^e` with
// m in [0.5, 1), so log(n) = ln(m) + e*ln(2) — computed exactly even
// for ints far too large for f64 (log(10**1000) must not be +inf).
pub fn log_frexp_int(n: &num_bigint::BigInt) -> (f64, f64) {
    let d = n.bits();
    let e = d as f64;
    let m = if d > 53 {
        let top = (n >> (d - 53)).to_u64().unwrap_or(0) as f64;
        top / 9007199254740992.0
    } else {
        n.to_f64().unwrap_or(0.0) / 2f64.powi(d as i32)
    };
    (m, e)
}

pub fn math_log2_value(v: &PyObjectRef) -> PyResult<f64> {
    let b = v.borrow();
    if let PyObject::Int(i) = &*b {
        if i.sign() == num_bigint::Sign::NoSign || i.sign() == num_bigint::Sign::Minus {
            return Err(PyError::value_error("expected a positive input"));
        }
        let (m, e) = log_frexp_int(i);
        return Ok(m.log2() + e);
    }
    let x = math_arg_f64(v).ok_or_else(|| PyError::type_error("a float is required"))?;
    if x <= 0.0 {
        return Err(PyError::value_error(format!(
            "expected a positive input, got {:?}",
            x
        )));
    }
    Ok(x.log2())
}

pub fn dl_fast_sum(a: f64, b: f64) -> (f64, f64) {
    let x = a + b;
    let y = (a - x) + b;
    (x, y)
}

pub fn vector_norm(vals: &[f64], max: f64) -> f64 {
    if max == 0.0 || vals.len() <= 1 {
        return max;
    }
    // frexp exponent of max (max = m * 2^max_e, m in [0.5, 1))
    let max_bits = max.to_bits();
    let max_e = (((max_bits >> 52) & 0x7ff) as i32) - 1022;
    if max_e < -1023 {
        // max is subnormal: scale up, recurse, scale back.
        let scaled: Vec<f64> = vals.iter().map(|v| v / f64::MIN_POSITIVE).collect();
        return f64::MIN_POSITIVE * vector_norm(&scaled, max / f64::MIN_POSITIVE);
    }
    let scale = crate::object::ldexp_f64(1.0, -max_e);
    let mut csum = 1.0f64;
    let mut frac1 = 0.0f64;
    let mut frac2 = 0.0f64;
    for v in vals {
        let x = v * scale; // lossless scaling; |x| < 1
        let (pr_hi, pr_lo) = dl_mul(x, x); // exact squaring
        let (sm_hi, sm_lo) = dl_fast_sum(csum, pr_hi); // |csum| >= |pr_hi|
        csum = sm_hi;
        frac1 += pr_lo;
        frac2 += sm_lo;
    }
    let mut h = (csum - 1.0 + (frac1 + frac2)).sqrt();
    // Differential correction: h ~= sqrt(h^2 + x) ~= h + x/(2h).
    let (pr_hi, pr_lo) = dl_mul(-h, h);
    let (sm_hi, sm_lo) = dl_fast_sum(csum, pr_hi);
    csum = sm_hi;
    frac1 += pr_lo;
    frac2 += sm_lo;
    let x = csum - 1.0 + (frac1 + frac2);
    h += x / (2.0 * h);
    h / scale
}

pub fn dl_sum(a: f64, b: f64) -> (f64, f64) {
    // Algorithm 3.1 (error-free transformation of a sum)
    let x = a + b;
    let z = x - a;
    let y = (a - (x - z)) + (b - z);
    (x, y)
}

pub fn dl_mul(a: f64, b: f64) -> (f64, f64) {
    let hi = a * b;
    let lo = a.mul_add(b, -hi);
    (hi, lo)
}

pub fn tl_fma(x: f64, y: f64, total: (f64, f64, f64)) -> (f64, f64, f64) {
    let (pr_hi, pr_lo) = dl_mul(x, y);
    let (sm_hi, sm_lo) = dl_sum(total.0, pr_hi);
    let (r1_hi, r1_lo) = dl_sum(total.1, pr_lo);
    let (r2_hi, r2_lo) = dl_sum(r1_hi, sm_lo);
    (sm_hi, r2_hi, total.2 + r1_lo + r2_lo)
}

pub fn tl_to_d(total: (f64, f64, f64)) -> f64 {
    let (last_hi, last_lo) = dl_sum(total.1, total.0);
    total.2 + last_lo + last_hi
}
