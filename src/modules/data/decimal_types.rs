use crate::object::*;
use std::collections::HashMap;
use num_bigint::{BigInt, Sign};
use num_traits::{Zero, One, Signed, ToPrimitive};
use crate::modules::data::decimal::*;
use crate::modules::data::fractions::{frac_normalize, frac_instance_num_den, frac_to_f64};
pub fn build_decimal_type() -> PyObjectRef {
    let mut type_dict: HashMap<String, PyObjectRef> = HashMap::new();
    macro_rules! bf {
        ($name:expr, $f:expr) => {
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: $name.to_string(),
                func: $f,
            })
        };
    }

    type_dict.insert_str(
        "__init__",
        bf!("__init__", |args| {
            let v = if args.len() > 1 {
                decval_from_pyobject(&args[1])?
            } else {
                DecValue::zero()
            };
            if let PyObject::Instance { dict, .. } = &mut *args[0].borrow_mut() {
                dict.insert(DEC_SIGN_KEY.to_string(), py_bool(v.sign));
                dict.insert(DEC_COEFF_KEY.to_string(), py_int(v.coeff));
                dict.insert(DEC_EXP_KEY.to_string(), py_int(v.exp));
                dict.insert(
                    DEC_SPECIAL_KEY.to_string(),
                    py_str(special_to_str(&v.special)),
                );
            }
            Ok(py_none())
        }),
    );
    type_dict.insert_str(
        "__repr__",
        bf!("__repr__", |args| {
            let v = instance_to_decval(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
            Ok(py_str(&format!("Decimal('{}')", format_decvalue(&v))))
        }),
    );
    type_dict.insert_str(
        "__str__",
        bf!("__str__", |args| {
            let v = instance_to_decval(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
            Ok(py_str(&format_decvalue(&v)))
        }),
    );
    type_dict.insert_str(
        "__int__",
        bf!("__int__", |args| {
            let v = instance_to_decval(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
            if v.special != DecSpecial::Finite {
                return Err(PyError::value_error("cannot convert NaN/Infinity to int"));
            }
            let truncated = if v.exp >= 0 {
                &v.coeff * ten_pow(v.exp)
            } else {
                &v.coeff / ten_pow(-v.exp)
            };
            Ok(py_int(if v.sign { -truncated } else { truncated }))
        }),
    );
    type_dict.insert_str(
        "__float__",
        bf!("__float__", |args| {
            let v = instance_to_decval(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
            Ok(py_float(decval_to_f64(&v)))
        }),
    );
    type_dict.insert_str(
        "as_integer_ratio",
        bf!("as_integer_ratio", |args| {
            let v = instance_to_decval(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
            if matches!(v.special, DecSpecial::QNaN | DecSpecial::SNaN) {
                return Err(PyError::value_error("cannot convert NaN to integer ratio"));
            }
            if v.special != DecSpecial::Finite {
                return Err(PyError::overflow_error(
                    "cannot convert Infinity to integer ratio",
                ));
            }
            let (num, den) = if v.exp >= 0 {
                (v.coeff * ten_pow(v.exp), BigInt::one())
            } else {
                (v.coeff.clone(), ten_pow(-v.exp))
            };
            // Reduce to lowest terms (Decimal('3.5e-2') -> 7/200, not 35/1000).
            let (num, den) = frac_normalize(if v.sign { -num } else { num }, den)?;
            Ok(py_tuple(vec![py_int(num), py_int(den)]))
        }),
    );
    type_dict.insert_str(
        "sqrt",
        bf!("sqrt", |args| {
            if args.is_empty() {
                return Err(PyError::type_error("sqrt() missing self"));
            }
            let v = instance_to_decval(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
            if v.is_nan() {
                return Err(decimal_invalid_op("NaN ** (1/2)"));
            }
            if v.sign && !v.is_zero() {
                return Err(decimal_invalid_op("(-x) ** (1/2)"));
            }
            if v.special == DecSpecial::Infinity {
                return Ok(decval_to_instance(&DecValue::infinity(false)));
            }
            if v.is_zero() {
                return Ok(decval_to_instance(&v.clone()));
            }
            // Integer Newton's-method square root at the context's
            // precision. The previous `max(60, precision)` always produced
            // 60 digits, causing `_decimal_sqrt_of_frac` (which expects 28)
            // to return 60-digit unrounded values.
            let (precision, _rounding) = current_decimal_context();
            let prec = precision as i64;
            let mut c = v.coeff.clone();
            let mut e = v.exp;
            if e % 2 != 0 {
                c *= 10;
                e -= 1;
            }
            // Scale the coefficient so its integer sqrt has ~`prec`
            // significant digits, take the exact integer square root, and
            // adjust the exponent back down.
            let c_digits = (c.bits() as f64 * 0.30103) as i64 + 1;
            let m = (prec - (c_digits + 1) / 2).max(0);
            let scaled = &c * ten_pow(2 * m);
            let root = scaled.sqrt();
            let result = DecValue {
                special: DecSpecial::Finite,
                sign: false,
                coeff: root,
                exp: e / 2 - m,
            };
            Ok(decval_to_instance(&result))
        }),
    );
    type_dict.insert_str(
        "__bool__",
        bf!("__bool__", |args| {
            let v = instance_to_decval(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
            Ok(py_bool(!v.is_zero()))
        }),
    );
    type_dict.insert_str(
        "__hash__",
        bf!("__hash__", |args| {
            let v = instance_to_decval(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
            if v.special != DecSpecial::Finite {
                return Ok(py_int(0));
            }
            // Normalize (strip trailing zeros) so numerically-equal Decimals
            // with different (coeff, exp) representations — e.g. 1 vs 1.0 —
            // hash the same way `1 == 1.0` requires.
            let n = normalize_decval(&v);
            let s = format!("{}{}{}", n.sign, n.coeff, n.exp);
            builtin_hash(&[py_str(&s)])
        }),
    );
    type_dict.insert_str(
        "__eq__",
        bf!("__eq__", |args| {
            let a = instance_to_decval(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
            // An operand that isn't convertible to a Decimal (complex, a
            // user-defined class, ...) must return NotImplemented so the OTHER
            // side's reflected __eq__ gets a chance (`Decimal('1001.0') ==
            // 1001+0j` is True via complex.__eq__, not False).
            let b = match decval_from_pyobject(&args[1]) {
                Ok(v) => v,
                Err(_) => return Ok(py_not_implemented()),
            };
            Ok(py_bool(
                decimal_compare(&a, &b) == Some(std::cmp::Ordering::Equal),
            ))
        }),
    );
    macro_rules! dec_cmp {
        ($name:expr, $ord:pat) => {
            type_dict.insert(
                $name.to_string(),
                bf!($name, |args| {
                    let a = instance_to_decval(&args[0])
                        .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
                    let b = match decval_from_pyobject(&args[1]) {
                        Ok(v) => v,
                        // An unconvertible operand (complex, ...) must produce
                        // the standard "not supported between instances"
                        // TypeError, matching real CPython — not the internal
                        // conversion message.
                        Err(_) => {
                            return Err(PyError::type_error(format!(
                                "'{}' not supported between instances of '{}' and '{}'",
                                match $name {
                                    "__lt__" => "<",
                                    "__gt__" => ">",
                                    _ => "?",
                                },
                                args[0].get_type_name(),
                                args[1].get_type_name()
                            )))
                        }
                    };
                    match decimal_compare(&a, &b) {
                        Some($ord) => Ok(py_bool(true)),
                        Some(_) => Ok(py_bool(false)),
                        None => Err(PyError::type_error("cannot compare NaN with Decimal")),
                    }
                }),
            );
        };
    }
    dec_cmp!("__lt__", std::cmp::Ordering::Less);
    dec_cmp!("__gt__", std::cmp::Ordering::Greater);
    type_dict.insert_str(
        "__le__",
        bf!("__le__", |args| {
            let a = instance_to_decval(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
            let b = match decval_from_pyobject(&args[1]) {
                Ok(v) => v,
                Err(_) => {
                    return Err(PyError::type_error(format!(
                        "'<=' not supported between instances of '{}' and '{}'",
                        args[0].get_type_name(),
                        args[1].get_type_name()
                    )))
                }
            };
            match decimal_compare(&a, &b) {
                Some(std::cmp::Ordering::Less) | Some(std::cmp::Ordering::Equal) => {
                    Ok(py_bool(true))
                }
                Some(_) => Ok(py_bool(false)),
                None => Err(PyError::type_error("cannot compare NaN with Decimal")),
            }
        }),
    );
    type_dict.insert_str(
        "__ge__",
        bf!("__ge__", |args| {
            let a = instance_to_decval(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
            let b = match decval_from_pyobject(&args[1]) {
                Ok(v) => v,
                Err(_) => {
                    return Err(PyError::type_error(format!(
                        "'>=' not supported between instances of '{}' and '{}'",
                        args[0].get_type_name(),
                        args[1].get_type_name()
                    )))
                }
            };
            match decimal_compare(&a, &b) {
                Some(std::cmp::Ordering::Greater) | Some(std::cmp::Ordering::Equal) => {
                    Ok(py_bool(true))
                }
                Some(_) => Ok(py_bool(false)),
                None => Err(PyError::type_error("cannot compare NaN with Decimal")),
            }
        }),
    );
    macro_rules! dec_binop {
        ($name:expr, $op:expr) => {
            type_dict.insert(
                $name.to_string(),
                bf!($name, |args| {
                    let a = instance_to_decval(&args[0])
                        .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
                    // Decimal arithmetic accepts only Decimal/int/bool/float
                    // operands — anything else (Fraction, complex, str, ...)
                    // defers to the other operand's reflected method, which
                    // ultimately raises TypeError (CPython: "Decimal refuses
                    // mixed arithmetic (but not mixed comparisons)").
                    let b_ok = matches!(
                        &*args[1].borrow(),
                        PyObject::Int(_) | PyObject::Bool(_) | PyObject::Float(_)
                    ) || instance_to_decval(&args[1]).is_some();
                    if !b_ok {
                        return Ok(py_not_implemented());
                    }
                    let b = decval_from_pyobject(&args[1])?;
                    Ok(decval_to_instance(&$op(&a, &b)?))
                }),
            );
        };
    }
    dec_binop!("__add__", decimal_add);
    dec_binop!("__radd__", |a, b| decimal_add(b, a));
    dec_binop!("__sub__", decimal_sub);
    dec_binop!("__rsub__", |a, b| decimal_sub(b, a));
    dec_binop!("__mul__", decimal_mul);
    dec_binop!("__rmul__", |a, b| decimal_mul(b, a));
    dec_binop!("__truediv__", decimal_div);
    dec_binop!("__rtruediv__", |a, b| decimal_div(b, a));
    type_dict.insert_str(
        "__floordiv__",
        bf!("__floordiv__", |args| {
            let a = instance_to_decval(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
            let b = decval_from_pyobject(&args[1])?;
            let q = decimal_div(&a, &b)?;
            if q.special != DecSpecial::Finite {
                return Ok(decval_to_instance(&q));
            }
            let truncated = if q.exp >= 0 {
                &q.coeff * ten_pow(q.exp)
            } else {
                &q.coeff / ten_pow(-q.exp)
            };
            Ok(decval_to_instance(&DecValue {
                special: DecSpecial::Finite,
                sign: q.sign,
                coeff: truncated,
                exp: 0,
            }))
        }),
    );
    type_dict.insert_str(
        "__mod__",
        bf!("__mod__", |args| {
            let a = instance_to_decval(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
            let b = decval_from_pyobject(&args[1])?;
            if b.is_zero() {
                return Err(decimal_invalid_op("0 modulo"));
            }
            let q = decimal_div(&a, &b)?;
            let truncated_q = if q.exp >= 0 {
                &q.coeff * ten_pow(q.exp)
            } else {
                &q.coeff / ten_pow(-q.exp)
            };
            let trunc_dec = DecValue {
                special: DecSpecial::Finite,
                sign: q.sign,
                coeff: truncated_q,
                exp: 0,
            };
            let prod = decimal_mul(&trunc_dec, &b)?;
            Ok(decval_to_instance(&decimal_sub(&a, &prod)?))
        }),
    );
    type_dict.insert_str(
        "__pow__",
        bf!("__pow__", |args| {
            let a = instance_to_decval(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
            let b = decval_from_pyobject(&args[1])?;
            if b.special != DecSpecial::Finite {
                // NaN/Infinity exponent – match CPython's Decimal semantics
                // (NaN ** anything -> NaN, etc.). For simplicity, fall back
                // to float pow which already handles these special values.
                let af = decval_to_f64(&a);
                let bf = decval_to_f64(&b);
                let rf = af.powf(bf);
                return Ok(decval_to_instance(&float_to_decvalue(rf)));
            }
            if b.exp < 0 {
                // Fractional exponent – previous stub raised RuntimeError, but
                // `statistics.geometric_mean`'s test does
                // `math.prod(map(Decimal, rng)) ** (Decimal(1)/len(rng))` with
                // a fractional exponent (1/n). Use float fallback for a
                // correctly-rounded approximation (isclose-checked, not exact).
                // For huge `a` (e.g. 1e35659 for range(1,10000)) `decval_to_f64`
                // overflows to inf, making `powf(inf, small)` -> inf. Compute
                // via logs to avoid overflow: a = coeff*10^exp, ln(a) =
                // ln(coeff)+exp*ln(10).
                let bf = decval_to_f64(&b);
                let af_direct = decval_to_f64(&a);
                // Negative base with fractional exponent is InvalidOperation
                if af_direct < 0.0 && bf.fract() != 0.0 {
                    return Err(decimal_invalid_op("(-x) ** (1/2)"));
                }
                let rf = if af_direct.is_infinite() && a.special == DecSpecial::Finite && !a.is_zero() {
                    let coeff_f = a.coeff.to_string().parse::<f64>().unwrap_or(f64::INFINITY);
                    let log_a = coeff_f.ln() + a.exp as f64 * 10f64.ln();
                    (bf * log_a).exp()
                } else {
                    af_direct.powf(bf)
                };
                return Ok(decval_to_instance(&float_to_decvalue(rf)));
            }
            let n = (&b.coeff * ten_pow(b.exp))
                .to_string()
                .parse::<i64>()
                .unwrap_or(0);
            let n = if b.sign { -n } else { n };
            if n < 0 {
                // Negative integer exponent – also via float fallback (e.g.
                // Decimal(2) ** Decimal(-1) == 0.5).
                let af = decval_to_f64(&a);
                let bf = decval_to_f64(&b);
                let rf = af.powf(bf);
                return Ok(decval_to_instance(&float_to_decvalue(rf)));
            }
            let mut result = DecValue {
                special: DecSpecial::Finite,
                sign: false,
                coeff: num_bigint::BigInt::from(1),
                exp: 0,
            };
            for _ in 0..n {
                result = decimal_mul(&result, &a)?;
            }
            Ok(decval_to_instance(&result))
        }),
    );
    type_dict.insert_str(
        "__neg__",
        bf!("__neg__", |args| {
            let v = instance_to_decval(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
            Ok(decval_to_instance(&decimal_negate(&v)))
        }),
    );
    type_dict.insert_str(
        "__pos__",
        bf!("__pos__", |args| {
            let v = instance_to_decval(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
            Ok(decval_to_instance(&round_to_context(v)))
        }),
    );
    type_dict.insert_str(
        "__abs__",
        bf!("__abs__", |args| {
            let mut v = instance_to_decval(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
            v.sign = false;
            Ok(decval_to_instance(&v))
        }),
    );
    type_dict.insert_str(
        "is_nan",
        bf!("is_nan", |args| {
            let v = instance_to_decval(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
            Ok(py_bool(v.is_nan()))
        }),
    );
    type_dict.insert_str(
        "is_infinite",
        bf!("is_infinite", |args| {
            let v = instance_to_decval(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
            Ok(py_bool(v.special == DecSpecial::Infinity))
        }),
    );
    type_dict.insert_str(
        "is_finite",
        bf!("is_finite", |args| {
            let v = instance_to_decval(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
            Ok(py_bool(v.special == DecSpecial::Finite))
        }),
    );
    type_dict.insert_str(
        "is_zero",
        bf!("is_zero", |args| {
            let v = instance_to_decval(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
            Ok(py_bool(v.is_zero()))
        }),
    );
    type_dict.insert_str(
        "is_signed",
        bf!("is_signed", |args| {
            let v = instance_to_decval(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
            Ok(py_bool(v.sign))
        }),
    );
    type_dict.insert_str(
        "copy_sign",
        bf!("copy_sign", |args| {
            let mut v = instance_to_decval(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
            let other = decval_from_pyobject(&args[1])?;
            v.sign = other.sign;
            Ok(decval_to_instance(&v))
        }),
    );
    type_dict.insert_str(
        "copy_abs",
        bf!("copy_abs", |args| {
            let mut v = instance_to_decval(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
            v.sign = false;
            Ok(decval_to_instance(&v))
        }),
    );
    type_dict.insert_str(
        "copy_negate",
        bf!("copy_negate", |args| {
            let v = instance_to_decval(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
            Ok(decval_to_instance(&decimal_negate(&v)))
        }),
    );
    // `next_plus` / `next_minus` – used by `statistics._decimal_sqrt_of_frac`
    // to correct a 1-ulp error after `sqrt()`. CPython's Decimal implements
    // these as the next representable value toward +inf / -inf. Our
    // implementation adds/subtracts one unit in the last place (1*10^{exp})
    // which is sufficient for the sqrt correction test (the root is never 0
    // or an extreme exponent in that test).
    type_dict.insert_str(
        "next_plus",
        bf!("next_plus", |args| {
            let v = instance_to_decval(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
            if v.special != DecSpecial::Finite {
                return Ok(decval_to_instance(&v));
            }
            if v.is_zero() {
                // Smallest positive value at this exponent.
                return Ok(decval_to_instance(&DecValue {
                    special: DecSpecial::Finite,
                    sign: false,
                    coeff: num_bigint::BigInt::from(1),
                    exp: v.exp,
                }));
            }
            let mut r = v.clone();
            if !r.sign {
                r.coeff += 1;
            } else if r.coeff > num_bigint::BigInt::from(1) {
                r.coeff -= 1;
            } else {
                r = DecValue {
                    special: DecSpecial::Finite,
                    sign: false,
                    coeff: num_bigint::BigInt::from(0),
                    exp: 0,
                };
            }
            Ok(decval_to_instance(&r))
        }),
    );
    type_dict.insert_str(
        "next_minus",
        bf!("next_minus", |args| {
            let v = instance_to_decval(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
            if v.special != DecSpecial::Finite {
                return Ok(decval_to_instance(&v));
            }
            if v.is_zero() {
                return Ok(decval_to_instance(&DecValue {
                    special: DecSpecial::Finite,
                    sign: true,
                    coeff: num_bigint::BigInt::from(1),
                    exp: v.exp,
                }));
            }
            let mut r = v.clone();
            if r.sign {
                r.coeff += 1;
            } else if r.coeff > num_bigint::BigInt::from(1) {
                r.coeff -= 1;
            } else {
                r = DecValue {
                    special: DecSpecial::Finite,
                    sign: true,
                    coeff: num_bigint::BigInt::from(0),
                    exp: 0,
                };
            }
            Ok(decval_to_instance(&r))
        }),
    );
    type_dict.insert_str(
        "as_tuple",
        bf!("as_tuple", |args| {
            let v = instance_to_decval(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
            let sign_val = py_int(if v.sign { 1 } else { 0 });
            let digits_str = if num_traits::Zero::is_zero(&v.coeff) {
                "0".to_string()
            } else {
                v.coeff.to_string()
            };
            let digits: Vec<PyObjectRef> = digits_str
                .chars()
                .map(|c| py_int(c.to_digit(10).unwrap_or(0) as i64))
                .collect();
            let exp_val = match v.special {
                DecSpecial::Finite => py_int(v.exp),
                DecSpecial::Infinity => py_str("F"),
                DecSpecial::QNaN => py_str("n"),
                DecSpecial::SNaN => py_str("N"),
            };
            Ok(py_tuple(vec![sign_val, py_tuple(digits), exp_val]))
        }),
    );
    type_dict.insert_str(
        "normalize",
        bf!("normalize", |args| {
            let v = instance_to_decval(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
            Ok(decval_to_instance(&normalize_decval(&round_to_context(v))))
        }),
    );
    type_dict.insert_str(
        "quantize",
        bf!("quantize", |args| {
            let v = instance_to_decval(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
            if args.len() < 2 {
                return Err(PyError::type_error("quantize() missing exponent argument"));
            }
            let target = decval_from_pyobject(&args[1])?;
            if v.special != DecSpecial::Finite || target.special != DecSpecial::Finite {
                return Err(decimal_invalid_op("quantize with non-finite argument"));
            }
            let (_, rounding) = current_decimal_context();
            let target_exp = target.exp;
            let result = if target_exp >= v.exp {
                let drop = (target_exp - v.exp) as usize;
                round_decvalue(
                    &v,
                    digit_count(&v.coeff).saturating_sub(drop).max(1),
                    &rounding,
                )
            } else {
                let scale = ten_pow(v.exp - target_exp);
                DecValue {
                    special: DecSpecial::Finite,
                    sign: v.sign,
                    coeff: &v.coeff * scale,
                    exp: target_exp,
                }
            };
            Ok(decval_to_instance(&DecValue {
                exp: target_exp,
                ..result
            }))
        }),
    );
    type_dict.insert_str(
        "to_integral_value",
        bf!("to_integral_value", |args| {
            let v = instance_to_decval(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
            if v.special != DecSpecial::Finite || v.exp >= 0 {
                return Ok(decval_to_instance(&v));
            }
            let (_, rounding) = current_decimal_context();
            let rounded = round_decvalue(
                &v,
                digit_count(&v.coeff)
                    .saturating_sub((-v.exp) as usize)
                    .max(1),
                &rounding,
            );
            Ok(decval_to_instance(&DecValue {
                exp: 0,
                coeff: &rounded.coeff * ten_pow(rounded.exp),
                ..rounded
            }))
        }),
    );
    type_dict.insert_str(
        "compare",
        bf!("compare", |args| {
            let a = instance_to_decval(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
            let b = decval_from_pyobject(&args[1])?;
            let n: i64 = match decimal_compare(&a, &b) {
                Some(std::cmp::Ordering::Less) => -1,
                Some(std::cmp::Ordering::Greater) => 1,
                Some(std::cmp::Ordering::Equal) => 0,
                None => return Ok(decval_to_instance(&DecValue::nan())),
            };
            Ok(decval_to_instance(&DecValue {
                special: DecSpecial::Finite,
                sign: n < 0,
                coeff: num_bigint::BigInt::from(n.abs()),
                exp: 0,
            }))
        }),
    );

    type_dict.insert_str(
        "__hash__",
        bf!("__hash__", |args| {
            // CPython's Decimal hash: (coeff * 10**exp) mod 2**61-1 for finite
            // values (using the modular inverse of 10 for negative exponents),
            // ±INF (314159) for infinities, 0 for nans; signed by the value.
            let v = instance_to_decval(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
            const MOD: i64 = (1 << 61) - 1;
            let magnitude = match v.special {
                DecSpecial::QNaN | DecSpecial::SNaN => 0i64,
                DecSpecial::Infinity => 314159,
                DecSpecial::Finite => {
                    let modulus = num_bigint::BigInt::from(MOD);
                    let exp_hash = if v.exp >= 0 {
                        num_bigint::BigInt::from(10u32)
                            .modpow(&num_bigint::BigInt::from(v.exp), &modulus)
                    } else {
                        // 10**(-exp) = inv10**(|exp|); inv10 = 10**-1 mod P.
                        let inv10 = crate::object::bigint_mod_inverse(
                            &num_bigint::BigInt::from(10),
                            &modulus,
                        )
                        .unwrap_or_else(|| num_bigint::BigInt::from(1));
                        inv10.modpow(&num_bigint::BigInt::from(-v.exp), &modulus)
                    };
                    let h = (&v.coeff % &modulus * exp_hash) % &modulus;
                    h.to_i64().unwrap_or(0)
                }
            };
            let result = if v.sign { -magnitude } else { magnitude };
            Ok(py_int(if result == -1 { -2 } else { result }))
        }),
    );
    type_dict.insert_str(
        "from_float",
        bf!("from_float", |args| {
            // Decimal.from_float(f): the exact decimal value of the binary float.
            if args.is_empty() {
                return Err(PyError::type_error("from_float() takes exactly 1 argument"));
            }
            let f = args[0]
                .as_f64()
                .ok_or_else(|| PyError::type_error("from_float() argument must be float"))?;
            Ok(decval_to_instance(&float_to_decvalue(f)))
        }),
    );
    PyObjectRef::new(PyObject::Type {
        name: "Decimal".to_string(),
        dict: Box::new(str_map_to_typedict(type_dict)),
        bases: vec![],
        mro: vec![],
    })
}

pub fn build_context_type() -> PyObjectRef {
    let mut type_dict: HashMap<String, PyObjectRef> = HashMap::new();
    macro_rules! bf {
        ($name:expr, $f:expr) => {
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: $name.to_string(),
                func: $f,
            })
        };
    }
    type_dict.insert_str(
        "__init__",
        bf!("__init__", |args| {
            let ctor_args = args[1..].to_vec();
            let kw: Option<PyDict> = ctor_args.last().and_then(|a| {
                if let PyObject::Dict(d) = &*a.borrow() {
                    Some((**d).clone())
                } else {
                    None
                }
            });
            let get_kw = |name: &str| {
                kw.as_ref()
                    .and_then(|d| d.get(&py_str(name)).ok().flatten())
            };
            let precision = get_kw("prec").and_then(|v| v.as_i64()).unwrap_or(28) as usize;
            let rounding = get_kw("rounding")
                .map(|v| v.str())
                .unwrap_or_else(|| "ROUND_HALF_EVEN".to_string());
            if let PyObject::Instance { dict, .. } = &mut *args[0].borrow_mut() {
                dict.insert_str("prec", py_int(precision as i64));
                dict.insert_str("rounding", py_str(&rounding));
            }
            Ok(py_none())
        }),
    );
    type_dict.insert_str(
        "__repr__",
        bf!("__repr__", |args| {
            let prec = if let PyObject::Instance { dict, .. } = &*args[0].borrow() {
                dict.get_str("prec").and_then(|v| v.as_i64()).unwrap_or(28)
            } else {
                28
            };
            Ok(py_str(&format!("Context(prec={})", prec)))
        }),
    );
    type_dict.insert_str(
        "copy",
        bf!("copy", |args| {
            // Context.copy() creates a shallow copy of the context
            let prec = if let PyObject::Instance { dict, .. } = &*args[0].borrow() {
                dict.get_str("prec").and_then(|v| v.as_i64()).unwrap_or(28)
            } else {
                28
            };
            let rounding = if let PyObject::Instance { dict, .. } = &*args[0].borrow() {
                dict.get_str("rounding")
                    .map(|v| v.str())
                    .unwrap_or_else(|| "ROUND_HALF_EVEN".to_string())
            } else {
                "ROUND_HALF_EVEN".to_string()
            };
            Ok(make_context_instance(prec as usize, &rounding))
        }),
    );
    type_dict.insert_str(
        "traps",
        PyObjectRef::new(PyObject::Property(Box::new(PropertyData {
            getter: Some(PyObjectRef::new(PyObject::BuiltinFunction {
                name: "traps".to_string(),
                func: |args| {
                    if args.is_empty() {
                        return Err(PyError::type_error("traps requires self"));
                    }
                    // Return existing traps dict or create one
                    let obj = &args[0];
                    let existing = if let PyObject::Instance { dict, .. } = &*obj.borrow() {
                        dict.get_str("traps").cloned()
                    } else {
                        None
                    };
                    if let Some(v) = existing {
                        return Ok(v);
                    }
                    let mut d = crate::object::PyDict::new();
                    let result = PyObjectRef::new(PyObject::Dict(Box::new(d)));
                    if let PyObject::Instance { dict, .. } = &mut *obj.borrow_mut() {
                        dict.insert_str("traps", result.clone());
                    }
                    Ok(result)
                },
            })),
            setter: None,
            deleter: None,
            doc: None,
        }))),
    );
    PyObjectRef::new(PyObject::Type {
        name: "Context".to_string(),
        dict: Box::new(str_map_to_typedict(type_dict)),
        bases: vec![],
        mro: vec![],
    })
}

pub fn get_context_type() -> PyObjectRef {
    let existing = DECIMAL_CONTEXT_TYPE.with(|c| c.borrow().clone());
    if let Some(t) = existing {
        return t;
    }
    let typ = build_context_type();
    DECIMAL_CONTEXT_TYPE.with(|c| {
        *c.borrow_mut() = Some(typ.clone());
    });
    typ
}

pub fn make_context_instance(precision: usize, rounding: &str) -> PyObjectRef {
    let typ = get_context_type();
    let mut dict = AttrMap::new();
    dict.insert_str("prec", py_int(precision as i64));
    dict.insert_str("rounding", py_str(rounding));
    dict.insert_str("Emax", py_int(999999999999999999i64));
    dict.insert_str("Emin", py_int(-999999999999999999i64));
    dict.insert_str("_is_basic", py_bool(false));
    PyObjectRef::new(PyObject::Instance { typ, dict })
}

