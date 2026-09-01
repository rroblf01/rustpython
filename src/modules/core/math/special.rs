use crate::object::*;
use std::collections::HashMap;
use num_traits::{Signed, ToPrimitive};

use super::helpers::{exact_fsum, math_arg_f64, math_int_value, tl_fma, tl_to_d};

/// Register the "additional math functions" group: ldexp, fsum, sumprod,
/// remainder, modf, frexp, ulp, nextafter, prod. These were the last ~400
/// lines of the original math.rs and form a natural extra/special group.
pub fn register_extra(d: &mut HashMap<String, PyObjectRef>) {
    macro_rules! math_func {
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

    math_func!("ldexp", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("ldexp() requires 2 arguments"));
        }
        let x = math_arg_f64(&args[0])
            .ok_or_else(|| PyError::type_error("argument must be a number"))?;
        let exp_big = math_int_value(&args[1])
            .map_err(|_| PyError::type_error("exponent must be an integer"))?;
        let exp = exp_big
            .to_i64()
            .map(|e| e.clamp(i32::MIN as i64, i32::MAX as i64))
            .unwrap_or_else(|| {
                // Exponent too big to fit i64: saturate to the extreme
                // (10**40 -> huge positive/negative -> inf/0.0).
                if exp_big.sign() == num_bigint::Sign::Minus {
                    i32::MIN as i64
                } else {
                    i32::MAX as i64
                }
            });
        let result = crate::object::ldexp_f64(x, exp as i32);
        if result.is_infinite() && x.is_finite() {
            return Err(PyError::overflow_error("math range error"));
        }
        Ok(py_float(result))
    });
    math_func!("fsum", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("fsum() requires an argument"));
        }
        // Any iterable (generator, map/filter, custom __iter__ ...).
        let items = collect_iterable(&args[0])?;
        let mut vals = Vec::with_capacity(items.len());
        for item in &items {
            let x = math_arg_f64(&item).ok_or_else(|| {
                PyError::type_error(format!(
                    "must be real number, not {}",
                    item.borrow().type_name()
                ))
            })?;
            // A huge INT that overflows to +inf (e.g. 10**1000) is an
            // OverflowError; a genuine float inf is handled by exact_fsum.
            if x.is_infinite() && matches!(&*item.borrow(), PyObject::Int(_)) {
                return Err(PyError::overflow_error("int too large to convert to float"));
            }
            vals.push(x);
        }
        Ok(py_float(exact_fsum(&vals)?))
    });

    // sumprod(p, q) — dot product of two equal-length iterables (added in
    // CPython 3.12), needed by real CPython's own `Lib/statistics.py`.
    math_func!("sumprod", |args| {
        if args.len() != 2 {
            return Err(PyError::type_error(
                "sumprod() takes exactly 2 positional arguments",
            ));
        }
        let p = collect_iterable(&args[0])?;
        let q = collect_iterable(&args[1])?;
        if p.len() != q.len() {
            return Err(PyError::value_error("inputs are not the same length"));
        }
        // Faithful port of CPython's math_sumprod_impl: three lanes.
        //  - int lane: exact products of two exact ints, accumulated in a
        //    C-long-sized integer; on overflow (or a non-int pair) it is
        //    finalized and disabled FOREVER.
        //  - float lane: float/int/bool pairs through the TripleLength
        //    accumulator (tl_fma); on a non-finite result or a non-float
        //    element it is finalized and disabled forever.
        //  - normal lane: real object `*` and `+` for whatever remains
        //    (Fraction/Decimal keep exact types; huge int x float raises
        //    OverflowError; inf + -inf is NaN).
        let as_i64 = |v: &PyObjectRef| -> Option<i64> {
            if let PyObject::Int(i) = &*v.borrow() {
                i.to_i64()
            } else {
                None
            }
        };
        let floatable = |v: &PyObjectRef| -> Option<f64> {
            match &*v.borrow() {
                PyObject::Int(i) => i.to_f64().filter(|f| f.is_finite()),
                _ => v.as_f64(),
            }
        };
        let mut total = py_int(0);
        let mut int_enabled = true;
        let mut int_total: i64 = 0;
        let mut int_in_use = false;
        let mut flt_enabled = true;
        let mut flt_total: (f64, f64, f64) = (0.0, 0.0, 0.0);
        let mut flt_in_use = false;
        for (a, b) in p.iter().zip(q.iter()) {
            let both_int = matches!(&*a.borrow(), PyObject::Int(_))
                && matches!(&*b.borrow(), PyObject::Int(_));
            if int_enabled {
                let prod = if both_int {
                    as_i64(a)
                        .zip(as_i64(b))
                        .and_then(|(ai, bi)| ai.checked_mul(bi))
                } else {
                    None
                };
                if let Some(prod) = prod {
                    if let Some(nxt) = int_total.checked_add(prod) {
                        int_total = nxt;
                        int_in_use = true;
                        continue;
                    }
                }
                // finalize int lane
                int_enabled = false;
                if int_in_use {
                    total = crate::object::py_add(&total, &py_int(int_total))?;
                    int_total = 0;
                    int_in_use = false;
                }
            }
            if flt_enabled {
                // CPython's float lane requires at least one exact FLOAT
                // operand (float*float, float*int, int*float); a pure
                // int*int pair never enters it, even after the int lane
                // overflowed (that's exactly how the -7.5 in
                // sumprod((-5,-5,10), (1.5, 2**62, 2**61)) gets lost: the
                // int products overflow a C long, flushing the float lane
                // and falling to ordinary float arithmetic).
                let p_is_float = matches!(&*a.borrow(), PyObject::Float(_));
                let q_is_float = matches!(&*b.borrow(), PyObject::Float(_));
                let nft = if p_is_float || q_is_float {
                    match (floatable(a), floatable(b)) {
                        (Some(fa), Some(fb)) => tl_fma(fa, fb, flt_total),
                        _ => (f64::NAN, 0.0, 0.0),
                    }
                } else {
                    (f64::NAN, 0.0, 0.0)
                };
                if nft.0.is_finite() {
                    flt_total = nft;
                    flt_in_use = true;
                    continue;
                }
                // finalize float lane
                flt_enabled = false;
                if flt_in_use {
                    total = crate::object::py_add(&total, &py_float(tl_to_d(flt_total)))?;
                    flt_total = (0.0, 0.0, 0.0);
                    flt_in_use = false;
                }
            }
            // normal lane
            let term = crate::object::py_mul(a, b)?;
            total = crate::object::py_add(&total, &term)?;
        }
        if int_in_use {
            total = crate::object::py_add(&total, &py_int(int_total))?;
        }
        if flt_in_use {
            total = crate::object::py_add(&total, &py_float(tl_to_d(flt_total)))?;
        }
        Ok(total)
    });
    math_func!("remainder", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("remainder() requires 2 arguments"));
        }
        let x = math_arg_f64(&args[0])
            .ok_or_else(|| PyError::type_error("argument must be a number"))?;
        let y = math_arg_f64(&args[1])
            .ok_or_else(|| PyError::type_error("argument must be a number"))?;
        if x.is_nan() {
            return Ok(py_float(f64::NAN));
        }
        if y.is_nan() {
            return Ok(py_float(f64::NAN));
        }
        if x.is_infinite() {
            return Err(PyError::value_error("math domain error"));
        }
        if y == 0.0 {
            return Err(PyError::value_error("math domain error"));
        }
        if y.is_infinite() {
            return Ok(py_float(x));
        }
        // Faithful port of CPython's m_remainder: reduce |x| mod |y| via
        // fmod (never overflows), compare against the complement c =
        // absy - m (Sterbenz-exact), and on an exact half choose the even
        // quotient. All steps stay within [0, |y|], so huge quotients
        // can't overflow the intermediate `y * round(x/y)`.
        let absx = x.abs();
        let absy = y.abs();
        let m = (absx % absy).abs();
        let c = absy - m;
        let r = if m < c {
            m
        } else if m > c {
            -c
        } else {
            m - 2.0 * ((0.5 * (absx - m)) % absy).abs()
        };
        Ok(py_float(f64::copysign(1.0, x) * r))
    });
    math_func!("modf", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("modf() requires an argument"));
        }
        let x = math_arg_f64(&args[0])
            .ok_or_else(|| PyError::type_error("argument must be a number"))?;
        if x.is_infinite() {
            return Ok(py_tuple(vec![py_float(f64::copysign(0.0, x)), py_float(x)]));
        }
        if x.is_nan() {
            return Ok(py_tuple(vec![py_float(f64::NAN), py_float(f64::NAN)]));
        }
        let frac = x.fract();
        let integer = x.trunc();
        Ok(py_tuple(vec![py_float(frac), py_float(integer)]))
    });
    math_func!("frexp", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("frexp() requires an argument"));
        }
        let x = math_arg_f64(&args[0])
            .ok_or_else(|| PyError::type_error("argument must be a number"))?;
        if x == 0.0 {
            return Ok(py_tuple(vec![py_float(0.0), py_int(0)]));
        }
        if x.is_infinite() || x.is_nan() {
            return Ok(py_tuple(vec![py_float(x), py_int(0)]));
        }
        let bits = x.to_bits();
        let biased_exp = ((bits >> 52) & 0x7ff) as i64;
        let normalized_exp = biased_exp - 1023;
        let mantissa_bits = bits & 0x000f_ffff_ffff_ffff;
        let sign_bit = bits & 0x8000_0000_0000_0000;
        // Reconstruct mantissa in range [0.5, 1)
        let fraction_bits = sign_bit | (0x3fe << 52) | mantissa_bits;
        let fraction = f64::from_bits(fraction_bits);
        // x = fraction * 2^exp with fraction in [0.5, 1): the fraction
        // reconstruction above divides by 2 (exponent 0x3fe = 1022), so
        // the reported exponent is biased_exp - 1023 + 1.
        Ok(py_tuple(vec![
            py_float(fraction),
            py_int(normalized_exp + 1),
        ]))
    });
    math_func!("ulp", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("ulp() requires an argument"));
        }
        let x = math_arg_f64(&args[0])
            .ok_or_else(|| PyError::type_error("argument must be a number"))?;
        // Calculate ULP: distance to next representable float
        if x.is_nan() {
            return Ok(py_float(f64::NAN));
        }
        if x.is_infinite() {
            return Ok(py_float(f64::INFINITY));
        }
        if x == 0.0 {
            // ulp(±0.0) is the smallest subnormal (CPython: 5e-324)
            return Ok(py_float(f64::from_bits(1)));
        }
        let abs = x.abs();
        // One ulp above `abs`; at the very top of the exponent range that
        // overflows to +inf (ulp(FLOAT_MAX) must still be the binade gap),
        // so measure downward instead.
        let up = f64::from_bits(abs.to_bits() + 1);
        let result = if up.is_infinite() {
            abs - f64::from_bits(abs.to_bits() - 1)
        } else {
            up - abs
        };
        Ok(py_float(result))
    });
    math_func!("nextafter", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("nextafter() requires 2 arguments"));
        }
        let x = math_arg_f64(&args[0])
            .ok_or_else(|| PyError::type_error("argument must be a number"))?;
        let y = math_arg_f64(&args[1])
            .ok_or_else(|| PyError::type_error("argument must be a number"))?;
        let steps = if args.len() >= 3 {
            let step_arg = &args[2];
            if let PyObject::Dict(kwargs) = &*step_arg.borrow() {
                match kwargs.get(&py_str("steps")) {
                    Ok(Some(v)) => math_int_value(&v)
                        .map_err(|_| PyError::type_error("steps argument must be an integer"))?
                        .to_i64()
                        .ok_or_else(|| PyError::overflow_error("steps too large"))?,
                    _ => 1,
                }
            } else {
                math_int_value(step_arg)
                    .map_err(|_| PyError::type_error("steps argument must be an integer"))?
                    .to_i64()
                    .ok_or_else(|| PyError::overflow_error("steps too large"))?
            }
        } else {
            1
        };
        if x.is_nan() || y.is_nan() {
            return Ok(py_float(f64::NAN));
        }
        if steps < 0 {
            return Err(PyError::value_error("steps must not be negative"));
        }
        if x == y {
            // CPython returns `y` (this also handles the -0.0 -> +0.0
            // crossing, where bit-level equality differs from numeric).
            return Ok(py_float(y));
        }
        // Monotonic signed ordering of IEEE-754 bit patterns: negative
        // floats map below 0 (i64::MIN - bits-as-i64), non-negative map
        // directly, so +/-inf are the range bounds and stepping the signed
        // value by one moves exactly one ulp toward y (correct across the
        // sign boundary and at ±0, unlike the naive bits+1/bits-1 trick).
        let to_ord = |bits: u64| -> i64 {
            let i = bits as i64;
            if i >= 0 {
                i
            } else {
                i64::MIN - i
            }
        };
        let from_ord = |o: i64| -> u64 {
            let i = if o >= 0 { o } else { i64::MIN - o };
            i as u64
        };
        let ord_neg_inf = to_ord(0xfff0_0000_0000_0000u64);
        let ord_pos_inf = to_ord(0x7ff0_0000_0000_0000u64);
        let dir: i64 = if y > x { 1 } else { -1 };
        let target = (to_ord(x.to_bits()) as i128) + (dir as i128) * (steps as i128);
        let target = target.clamp(ord_neg_inf as i128, ord_pos_inf as i128);
        if target == 0 && x.to_bits() != 0 {
            // Landing exactly on the zero boundary (a subnormal stepping
            // toward 0): preserve x's sign — CPython returns ±0.0 matching
            // the side x came from.
            return Ok(py_float(f64::copysign(0.0, x)));
        }
        Ok(py_float(f64::from_bits(from_ord(target as i64))))
    });
    math_func!("prod", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("prod() requires an argument"));
        }
        let start = if args.len() > 1 {
            match &*args[1].borrow() {
                PyObject::Dict(kwargs) => match kwargs.get(&py_str("start")) {
                    Ok(Some(v)) => v.clone(),
                    _ => py_int(1),
                },
                // start is keyword-only; a second POSITIONAL arg is an error.
                _ => {
                    return Err(PyError::type_error(
                        "prod() takes at most 1 positional argument",
                    ))
                }
            }
        } else {
            py_int(1)
        };
        let items = collect_iterable(&args[0])?;
        // prod is plain repeated multiplication, so delegate to the real
        // object `*` (handles int/float/Fraction/Decimal and propagates
        // any error from user __mul__/__rmul__, e.g. RuntimeError).
        let mut result = start;
        for item in &items {
            result = crate::object::py_mul(&result, item)?;
        }
        Ok(result)
    });
}
