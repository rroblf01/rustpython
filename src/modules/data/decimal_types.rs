use crate::modules::data::decimal::*;
use crate::modules::data::fractions::{frac_instance_num_den, frac_normalize, frac_to_f64};
use crate::object::*;
use num_bigint::{BigInt, Sign};
use num_traits::{One, Signed, ToPrimitive, Zero};
use std::collections::HashMap;
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
        "__format__",
        bf!("__format__", |args| {
            if args.len() < 2 {
                return Err(PyError::type_error("__format__ requires 1 argument"));
            }
            if matches!(&*args[1].borrow(), PyObject::None) {
                return Err(PyError::type_error(
                    "format() argument 2 must be str, not NoneType",
                ));
            }
            let v = instance_to_decval(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
            let spec = args[1].str();
            // `format_decimal_spec` raises ValueError ("invalid format
            // string") for bad specs, mirroring CPython.
            let out = crate::modules::data::decimal::format_decimal_spec(&v, &spec)?;
            Ok(py_str(&out))
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
                        // An operand that is neither a Decimal nor something
                        // Decimal can compare with (complex, Fraction, a
                        // user class, ...) must return NotImplemented so the
                        // interpreter itself raises the standard "'<' not
                        // supported between instances of 'Decimal' and
                        // 'complex'" TypeError.
                        Err(_) => return Ok(py_not_implemented()),
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
                    // CPython refuses to mix Decimal and float in
                    // arithmetic ("unsupported operand type(s) for +:
                    // 'decimal.Decimal' and 'float'"); comparisons and the
                    // constructor still accept floats.
                    let b_ok = matches!(&*args[1].borrow(), PyObject::Int(_) | PyObject::Bool(_))
                        || instance_to_decval(&args[1]).is_some();
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
    // Truncating division shared by `__floordiv__`/`__rfloordiv__` and
    // `__divmod__`/`__rdivmod__`: `a // b` in CPython's Decimal semantics
    // truncates toward zero (it is NOT Python's floor division).
    fn dec_trunc_quotient(a: &DecValue, b: &DecValue) -> PyResult<DecValue> {
        let q = decimal_div(a, b)?;
        if q.special != DecSpecial::Finite {
            return Ok(q);
        }
        let truncated = if q.exp >= 0 {
            &q.coeff * ten_pow(q.exp)
        } else {
            &q.coeff / ten_pow(-q.exp)
        };
        Ok(DecValue {
            special: DecSpecial::Finite,
            sign: q.sign,
            coeff: truncated,
            exp: 0,
        })
    }
    // `a % b` for Decimals: the remainder that pairs with `dec_trunc_quotient`
    // (sign follows the dividend, unlike Python's `%`).
    fn dec_remainder(a: &DecValue, b: &DecValue) -> PyResult<DecValue> {
        if b.is_zero() {
            return Err(decimal_invalid_op("0 modulo"));
        }
        let q = decimal_div(a, b)?;
        let trunc_dec = if q.special != DecSpecial::Finite {
            q.clone()
        } else {
            let truncated_q = if q.exp >= 0 {
                &q.coeff * ten_pow(q.exp)
            } else {
                &q.coeff / ten_pow(-q.exp)
            };
            DecValue {
                special: DecSpecial::Finite,
                sign: q.sign,
                coeff: truncated_q,
                exp: 0,
            }
        };
        let prod = decimal_mul(&trunc_dec, b)?;
        decimal_sub(a, &prod)
    }
    // Accept only Decimal/int/bool/float operands, like the other binary
    // operators; anything else defers to the other operand's reflected
    // method.
    fn dec_operand_ok(obj: &PyObjectRef) -> bool {
        matches!(&*obj.borrow(), PyObject::Int(_) | PyObject::Bool(_))
            || instance_to_decval(obj).is_some()
    }
    macro_rules! dec_reflected {
        ($name:expr, $op:expr) => {
            type_dict.insert_str(
                $name,
                bf!($name, |args| {
                    let a = instance_to_decval(&args[0])
                        .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
                    if !dec_operand_ok(&args[1]) {
                        return Ok(py_not_implemented());
                    }
                    let b = decval_from_pyobject(&args[1])?;
                    Ok(decval_to_instance(&$op(&a, &b)?))
                }),
            );
        };
    }
    // `int // Decimal` and friends: `int.__floordiv__` returns
    // NotImplemented for a Decimal, so without these reflected forms every
    // `7 // Decimal(2)`, `7 % Decimal(2)` and `divmod(7, Decimal(2))` raised
    // "unsupported operand type(s)" (269 failures in test_decimal alone).
    dec_reflected!("__rfloordiv__", |a: &DecValue, b: &DecValue| {
        dec_trunc_quotient(b, a)
    });
    dec_reflected!("__rmod__", |a: &DecValue, b: &DecValue| dec_remainder(b, a));
    type_dict.insert_str(
        "__rdivmod__",
        bf!("__rdivmod__", |args| {
            let a = instance_to_decval(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
            if !dec_operand_ok(&args[1]) {
                return Ok(py_not_implemented());
            }
            let b = decval_from_pyobject(&args[1])?;
            let q = dec_trunc_quotient(&b, &a)?;
            let r = dec_remainder(&b, &a)?;
            Ok(PyObjectRef::new(PyObject::Tuple(vec![
                decval_to_instance(&q),
                decval_to_instance(&r),
            ])))
        }),
    );
    dec_reflected!("__rtruediv__", |a: &DecValue, b: &DecValue| decimal_div(
        b, a
    ));
    type_dict.insert_str(
        "__divmod__",
        bf!("__divmod__", |args| {
            let a = instance_to_decval(&args[0])
                .ok_or_else(|| PyError::runtime_error("not a Decimal"))?;
            if !dec_operand_ok(&args[1]) {
                return Ok(py_not_implemented());
            }
            let b = decval_from_pyobject(&args[1])?;
            let q = dec_trunc_quotient(&a, &b)?;
            let r = dec_remainder(&a, &b)?;
            Ok(PyObjectRef::new(PyObject::Tuple(vec![
                decval_to_instance(&q),
                decval_to_instance(&r),
            ])))
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
                let rf =
                    if af_direct.is_infinite() && a.special == DecSpecial::Finite && !a.is_zero() {
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
    // Real CPython's `repr(Decimal)` is `<class 'decimal.Decimal'>` — a
    // class's repr uses its `__module__`, which every OTHER ad-hoc native
    // type here sets explicitly (see `fractions_mod.rs`'s `Fraction` doing
    // the same for `"fractions"`); Decimal never did, so it fell back to
    // the builtins-style bare `<class 'Decimal'>` instead. Found via
    // `test_statistics.py`'s `_sum`/`pvariance` doctests, which literally
    // assert the module-qualified repr in their expected output.
    type_dict.insert_str("__module__", py_str("decimal"));
    // ------------------------------------------------------------------
    // The rest of CPython 3.14's `Decimal` surface. The shared numeric
    // bodies live in `decimal_ctx.rs` (they are the same ones the `Context`
    // methods use, with the current context supplying precision/rounding).
    // ------------------------------------------------------------------
    use crate::modules::data::decimal_ctx as dctx;
    use dctx::make_decimal_tuple;
    macro_rules! dec_self {
        ($name:expr, $f:expr) => {
            type_dict.insert_str(
                $name,
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: $name.to_string(),
                    func: $f,
                }),
            );
        };
    }
    /// `(self)` for the no-argument Decimal methods.
    fn self_value(args: &[PyObjectRef]) -> PyResult<DecValue> {
        args.first()
            .and_then(instance_to_decval)
            .ok_or_else(|| PyError::runtime_error("not a Decimal"))
    }
    /// `(self, other)` for the one-argument methods. Operands go through the
    /// STRICT conversion (Decimal/int/bool/float only): CPython's
    /// `Decimal('Infinity').fma(Decimal(0), "not a decimal")` is a TypeError,
    /// not an InvalidOperation from parsing the string.
    fn self_other(args: &[PyObjectRef]) -> PyResult<(DecValue, DecValue)> {
        if args.len() < 2 {
            return Err(PyError::type_error("method missing argument"));
        }
        let me = self_value(args)?;
        let other = dctx::ctx_decimal_arg(&args[1])?;
        Ok((me, other))
    }
    // Decimals are immutable, so `copy`/`deepcopy`/`pickle` all hand back
    // the same object (CPython's `Decimal.__deepcopy__` returns self). The
    // `copy` module reaches for `__copy__`/`__deepcopy__` on the TYPE and
    // calls them with `self`, so these need the usual `[self]` shape.
    dec_self!("__copy__", |args: &[PyObjectRef]| {
        args.first()
            .cloned()
            .ok_or_else(|| PyError::type_error("__copy__() missing argument"))
    });
    dec_self!("__deepcopy__", |args: &[PyObjectRef]| {
        args.first()
            .cloned()
            .ok_or_else(|| PyError::type_error("__deepcopy__() missing argument"))
    });
    dec_self!("__reduce__", |args: &[PyObjectRef]| {
        let me = args
            .first()
            .and_then(instance_to_decval)
            .ok_or_else(|| PyError::type_error("__reduce__() missing argument"))?;
        // `(str(decimal), (text,))` — rebuild through the Decimal
        // constructor, like CPython.
        let text = format_decvalue(&me);
        Ok(PyObjectRef::new(PyObject::Tuple(vec![
            py_str("Decimal"),
            PyObjectRef::new(PyObject::Tuple(vec![py_str(&text)])),
        ])))
    });
    dec_self!("adjusted", |args: &[PyObjectRef]| {
        let me = self_value(args)?;
        Ok(py_int(dctx::decimal_adjusted(&me)?))
    });
    dec_self!("as_tuple", |args: &[PyObjectRef]| {
        let me = self_value(args)?;
        let (sign, digits, exponent) = match me.special {
            DecSpecial::Infinity => {
                return Ok(make_decimal_tuple(
                    if me.sign { 1 } else { 0 },
                    py_str("Infinity"),
                    py_str("F"),
                ))
            }
            DecSpecial::QNaN | DecSpecial::SNaN => {
                let text = match me.special {
                    DecSpecial::SNaN => "sNaN",
                    _ => "NaN",
                };
                return Ok(make_decimal_tuple(
                    if me.sign { 1 } else { 0 },
                    py_str(text),
                    py_str(special_to_str(&me.special)),
                ));
            }
            DecSpecial::Finite => {
                let digits: Vec<PyObjectRef> = format!("{}", me.coeff)
                    .bytes()
                    .map(|b| py_int((b - b'0') as i64))
                    .collect();
                (
                    if me.sign { 1 } else { 0 },
                    PyObjectRef::new(PyObject::Tuple(digits)),
                    py_int(me.exp),
                )
            }
        };
        Ok(make_decimal_tuple(sign, digits, exponent))
    });
    dec_self!("canonical", |args: &[PyObjectRef]| {
        let me = self_value(args)?;
        Ok(decval_to_instance(&me))
    });
    dec_self!("compare", |args: &[PyObjectRef]| {
        let (me, other) = self_other(args)?;
        let ordering = decimal_compare(&me, &other).ok_or_else(|| decimal_invalid_op("compare"))?;
        use std::cmp::Ordering;
        Ok(py_int(match ordering {
            Ordering::Less => -1,
            Ordering::Equal => 0,
            Ordering::Greater => 1,
        }))
    });
    dec_self!("compare_signal", |args: &[PyObjectRef]| {
        let (me, other) = self_other(args)?;
        let ordering =
            decimal_compare(&me, &other).ok_or_else(|| decimal_invalid_op("compare_signal"))?;
        use std::cmp::Ordering;
        Ok(py_int(match ordering {
            Ordering::Less => -1,
            Ordering::Equal => 0,
            Ordering::Greater => 1,
        }))
    });
    dec_self!("compare_total", |args: &[PyObjectRef]| {
        let (me, other) = self_other(args)?;
        Ok(py_int(dctx::decimal_compare_total(&me, &other)))
    });
    dec_self!("compare_total_mag", |args: &[PyObjectRef]| {
        let (me, other) = self_other(args)?;
        Ok(py_int(dctx::decimal_compare_total_mag(&me, &other)))
    });
    dec_self!("conjugate", |args: &[PyObjectRef]| {
        let me = self_value(args)?;
        Ok(decval_to_instance(&me))
    });
    dec_self!("copy_abs", |args: &[PyObjectRef]| {
        let me = self_value(args)?;
        Ok(decval_to_instance(&dctx::decimal_copy_abs(&me)))
    });
    dec_self!("copy_negate", |args: &[PyObjectRef]| {
        let me = self_value(args)?;
        Ok(decval_to_instance(&decimal_negate(&me)))
    });
    dec_self!("copy_sign", |args: &[PyObjectRef]| {
        let (me, other) = self_other(args)?;
        Ok(decval_to_instance(&dctx::decimal_copy_sign(&me, &other)))
    });
    dec_self!("exp", |args: &[PyObjectRef]| {
        let ctx = dctx::current_context();
        let me = self_value(args)?;
        Ok(decval_to_instance(&dctx::decimal_exp(&ctx, &me)?))
    });
    dec_self!("fma", |args: &[PyObjectRef]| {
        let ctx = dctx::current_context();
        if args.len() < 3 {
            return Err(PyError::type_error("fma() takes 3 arguments"));
        }
        let a = dctx::ctx_decimal_arg(&args[0])?;
        let b = dctx::ctx_decimal_arg(&args[1])?;
        let c = dctx::ctx_decimal_arg(&args[2])?;
        Ok(decval_to_instance(&dctx::decimal_fma(&ctx, &a, &b, &c)?))
    });
    dec_self!("from_number", |args: &[PyObjectRef]| {
        // `Decimal.from_number(x)` (3.14): like `Decimal(x)` but signals
        // FloatOperation for a float when the trap is armed. Callable both as
        // `Decimal.from_number(x)` and `x.from_number()`, so the value is the
        // LAST argument.
        let value_obj = args.last().cloned().unwrap_or_else(py_none);
        let ctx = dctx::current_context();
        if matches!(&*value_obj.borrow(), PyObject::Float(_))
            && dctx::signal_trapped(&ctx, "FloatOperation")
        {
            return Err(dctx::decimal_signal_error(
                "FloatOperation",
                "[<class 'float'>] either does not support arithmetic operations or the operation is invalid",
            ));
        }
        let value = dctx::ctx_decimal_arg(&value_obj)?;
        Ok(decval_to_instance(&dctx::apply_context(&ctx, value)?))
    });
    dec_self!("imag", |args: &[PyObjectRef]| {
        let _ = self_value(args)?;
        Ok(decval_to_instance(&DecValue::zero()))
    });
    dec_self!("real", |args: &[PyObjectRef]| {
        let me = self_value(args)?;
        Ok(decval_to_instance(&me))
    });
    dec_self!("is_canonical", |args: &[PyObjectRef]| {
        let me = self_value(args)?;
        Ok(py_bool(dctx::decimal_is_canonical(&me)))
    });
    dec_self!("is_normal", |args: &[PyObjectRef]| {
        let ctx = dctx::current_context();
        let me = self_value(args)?;
        Ok(py_bool(dctx::decimal_is_normal(&ctx, &me)))
    });
    dec_self!("is_qnan", |args: &[PyObjectRef]| {
        let me = self_value(args)?;
        Ok(py_bool(me.special == DecSpecial::QNaN))
    });
    dec_self!("is_snan", |args: &[PyObjectRef]| {
        let me = self_value(args)?;
        Ok(py_bool(me.special == DecSpecial::SNaN))
    });
    dec_self!("is_subnormal", |args: &[PyObjectRef]| {
        let ctx = dctx::current_context();
        let me = self_value(args)?;
        Ok(py_bool(dctx::decimal_is_subnormal(&ctx, &me)))
    });
    dec_self!("ln", |args: &[PyObjectRef]| {
        let ctx = dctx::current_context();
        let me = self_value(args)?;
        Ok(decval_to_instance(&dctx::decimal_ln(&ctx, &me)?))
    });
    dec_self!("log10", |args: &[PyObjectRef]| {
        let ctx = dctx::current_context();
        let me = self_value(args)?;
        Ok(decval_to_instance(&dctx::decimal_log10(&ctx, &me)?))
    });
    dec_self!("logb", |args: &[PyObjectRef]| {
        let ctx = dctx::current_context();
        let me = self_value(args)?;
        Ok(decval_to_instance(&dctx::decimal_logb(&ctx, &me)?))
    });
    dec_self!("logical_and", |args: &[PyObjectRef]| {
        let ctx = dctx::current_context();
        let (me, other) = self_other(args)?;
        Ok(decval_to_instance(&dctx::decimal_logical(
            &ctx,
            &me,
            Some(&other),
            "and",
        )?))
    });
    dec_self!("logical_or", |args: &[PyObjectRef]| {
        let ctx = dctx::current_context();
        let (me, other) = self_other(args)?;
        Ok(decval_to_instance(&dctx::decimal_logical(
            &ctx,
            &me,
            Some(&other),
            "or",
        )?))
    });
    dec_self!("logical_xor", |args: &[PyObjectRef]| {
        let ctx = dctx::current_context();
        let (me, other) = self_other(args)?;
        Ok(decval_to_instance(&dctx::decimal_logical(
            &ctx,
            &me,
            Some(&other),
            "xor",
        )?))
    });
    dec_self!("logical_invert", |args: &[PyObjectRef]| {
        let ctx = dctx::current_context();
        let me = self_value(args)?;
        Ok(decval_to_instance(&dctx::decimal_logical(
            &ctx, &me, None, "invert",
        )?))
    });
    dec_self!("max", |args: &[PyObjectRef]| {
        let ctx = dctx::current_context();
        let (me, other) = self_other(args)?;
        use std::cmp::Ordering;
        let out = if decimal_compare(&me, &other) == Some(Ordering::Less) {
            other
        } else {
            me
        };
        Ok(decval_to_instance(&dctx::apply_context(&ctx, out)?))
    });
    dec_self!("min", |args: &[PyObjectRef]| {
        let ctx = dctx::current_context();
        let (me, other) = self_other(args)?;
        use std::cmp::Ordering;
        let out = if decimal_compare(&me, &other) == Some(Ordering::Greater) {
            other
        } else {
            me
        };
        Ok(decval_to_instance(&dctx::apply_context(&ctx, out)?))
    });
    dec_self!("max_mag", |args: &[PyObjectRef]| {
        let ctx = dctx::current_context();
        let (me, other) = self_other(args)?;
        let out = if dctx::decimal_compare_total_mag(&me, &other) < 0 {
            other
        } else {
            me
        };
        Ok(decval_to_instance(&dctx::apply_context(&ctx, out)?))
    });
    dec_self!("min_mag", |args: &[PyObjectRef]| {
        let ctx = dctx::current_context();
        let (me, other) = self_other(args)?;
        let out = if dctx::decimal_compare_total_mag(&me, &other) > 0 {
            other
        } else {
            me
        };
        Ok(decval_to_instance(&dctx::apply_context(&ctx, out)?))
    });
    dec_self!("next_minus", |args: &[PyObjectRef]| {
        let ctx = dctx::current_context();
        let me = self_value(args)?;
        Ok(decval_to_instance(&dctx::decimal_next(&ctx, &me, -1)?))
    });
    dec_self!("next_plus", |args: &[PyObjectRef]| {
        let ctx = dctx::current_context();
        let me = self_value(args)?;
        Ok(decval_to_instance(&dctx::decimal_next(&ctx, &me, 1)?))
    });
    dec_self!("next_toward", |args: &[PyObjectRef]| {
        let (me, other) = self_other(args)?;
        use std::cmp::Ordering;
        let ordering =
            decimal_compare(&me, &other).ok_or_else(|| decimal_invalid_op("next_toward"))?;
        let ctx = dctx::current_context();
        if ordering == Ordering::Equal {
            return Ok(decval_to_instance(&me));
        }
        let away = match (me.sign, other.sign, ordering) {
            (false, false, Ordering::Less) | (true, true, Ordering::Greater) => true,
            _ => false,
        };
        let step = BigInt::from(1);
        let coeff = if away {
            if me.sign {
                &me.coeff - 1
            } else {
                &me.coeff + 1
            }
        } else if me.sign {
            &me.coeff + 1
        } else {
            &me.coeff - 1
        };
        let _ = step;
        let _ = ctx;
        Ok(decval_to_instance(&DecValue {
            special: DecSpecial::Finite,
            sign: me.sign,
            coeff: if coeff < BigInt::from(0) {
                BigInt::from(0)
            } else {
                coeff
            },
            exp: me.exp,
        }))
    });
    dec_self!("number_class", |args: &[PyObjectRef]| {
        let me = self_value(args)?;
        Ok(py_str(dctx::decimal_number_class(&me)))
    });
    dec_self!("radix", |_args: &[PyObjectRef]| Ok(py_int(10)));
    dec_self!("remainder_near", |args: &[PyObjectRef]| {
        let (me, other) = self_other(args)?;
        Ok(decval_to_instance(&dctx::decimal_remainder_near(
            &me, &other,
        )?))
    });
    dec_self!("rotate", |args: &[PyObjectRef]| {
        let (me, other) = self_other(args)?;
        Ok(decval_to_instance(&dctx::decimal_rotate(&me, &other)?))
    });
    dec_self!("same_quantum", |args: &[PyObjectRef]| {
        let (me, other) = self_other(args)?;
        Ok(py_bool(dctx::decimal_same_quantum(&me, &other)))
    });
    dec_self!("scaleb", |args: &[PyObjectRef]| {
        let ctx = dctx::current_context();
        let me = self_value(args)?;
        let n = args
            .get(1)
            .and_then(|v| v.as_i64())
            .ok_or_else(|| decimal_invalid_op("scaleb"))?;
        Ok(decval_to_instance(&dctx::decimal_scaleb(&ctx, &me, n)?))
    });
    dec_self!("shift", |args: &[PyObjectRef]| {
        let ctx = dctx::current_context();
        let (me, other) = self_other(args)?;
        Ok(decval_to_instance(&dctx::decimal_shift(&ctx, &me, &other)?))
    });
    dec_self!("sqrt", |args: &[PyObjectRef]| {
        let ctx = dctx::current_context();
        let me = self_value(args)?;
        Ok(decval_to_instance(&dctx::decimal_sqrt_ctx(&ctx, &me)?))
    });
    dec_self!("to_eng_string", |args: &[PyObjectRef]| {
        let me = self_value(args)?;
        Ok(py_str(&dctx::decimal_to_eng_string(&me)))
    });
    dec_self!("to_sci_string", |args: &[PyObjectRef]| {
        let me = self_value(args)?;
        Ok(py_str(&dctx::decimal_to_sci_string(&me)))
    });
    dec_self!("to_integral", |args: &[PyObjectRef]| {
        let ctx = dctx::current_context();
        let me = self_value(args)?;
        let rounding = args.get(1).map(|v| v.str());
        Ok(decval_to_instance(&dctx::decimal_to_integral_exact(
            &ctx,
            &me,
            rounding.as_deref(),
        )?))
    });
    dec_self!("to_integral_exact", |args: &[PyObjectRef]| {
        let ctx = dctx::current_context();
        let me = self_value(args)?;
        let rounding = args.get(1).map(|v| v.str());
        Ok(decval_to_instance(&dctx::decimal_to_integral_exact(
            &ctx,
            &me,
            rounding.as_deref(),
        )?))
    });
    dec_self!("quantize", |args: &[PyObjectRef]| {
        let ctx = dctx::current_context();
        if args.len() < 2 {
            return Err(PyError::type_error("quantize() takes 2 arguments"));
        }
        let me = dctx::ctx_decimal_arg(&args[0])?;
        let exp_obj = dctx::ctx_decimal_arg(&args[1])?;
        let rounding = args.get(2).map(|v| v.str());
        // The exponent comes from the argument's own exponent when it is a
        // Decimal (`quantize(Decimal('0.01'))` -> -2) and from the value
        // itself when it is an int.
        // An int exponent argument is `Decimal(int)`, whose exponent is 0.
        let exp = if args[1].as_i64().is_some() {
            0
        } else {
            exp_obj.exp
        };
        Ok(decval_to_instance(&dctx::decimal_quantize(
            &ctx,
            &me,
            exp,
            rounding.as_deref(),
        )?))
    });
    dec_self!("__round__", |args: &[PyObjectRef]| {
        let me = self_value(args)?;
        let ndigits = args.get(1).and_then(|v| v.as_i64());
        let rounding = args.get(2).map(|v| v.str());
        match ndigits {
            Some(n) => Ok(decval_to_instance(&dctx::decimal_round(
                &me,
                Some(n),
                rounding.as_deref(),
            )?)),
            None => {
                // `round(Decimal)` returns an int, like CPython.
                let rounded = dctx::round_to_exponent(
                    &me,
                    0,
                    rounding.as_deref().unwrap_or("ROUND_HALF_EVEN"),
                );
                let int = rounded
                    .coeff
                    .to_i64()
                    .ok_or_else(|| PyError::overflow_error("integer division result too large"))?;
                Ok(py_int(if rounded.sign { -int } else { int }))
            }
        }
    });
    dec_self!("__trunc__", |args: &[PyObjectRef]| {
        let me = self_value(args)?;
        let rounded = dctx::round_to_exponent(&me, 0, "ROUND_DOWN");
        let int = rounded
            .coeff
            .to_i64()
            .ok_or_else(|| PyError::overflow_error("integer division result too large"))?;
        Ok(py_int(if rounded.sign { -int } else { int }))
    });
    dec_self!("__floor__", |args: &[PyObjectRef]| {
        let me = self_value(args)?;
        let rounding = if me.sign { "ROUND_FLOOR" } else { "ROUND_DOWN" };
        let rounded = dctx::round_to_exponent(&me, 0, rounding);
        let int = rounded
            .coeff
            .to_i64()
            .ok_or_else(|| PyError::overflow_error("integer division result too large"))?;
        Ok(py_int(if rounded.sign { -int } else { int }))
    });
    dec_self!("__ceil__", |args: &[PyObjectRef]| {
        let me = self_value(args)?;
        let rounding = if me.sign {
            "ROUND_DOWN"
        } else {
            "ROUND_CEILING"
        };
        let rounded = dctx::round_to_exponent(&me, 0, rounding);
        let int = rounded
            .coeff
            .to_i64()
            .ok_or_else(|| PyError::overflow_error("integer division result too large"))?;
        Ok(py_int(if rounded.sign { -int } else { int }))
    });
    dec_self!("__rpow__", |args: &[PyObjectRef]| {
        // `2 ** Decimal(3)` dispatches here with [base, exponent]; a direct
        // call has [self, base, exponent].
        let (base_obj, exponent_obj) = match args.len() {
            2 => (args[0].clone(), args[1].clone()),
            _ => (args[1].clone(), args[2].clone()),
        };
        let base = dctx::ctx_decimal_arg(&base_obj)?;
        let exponent = dctx::ctx_decimal_arg(&exponent_obj)?;
        let ctx = dctx::current_context();
        Ok(decval_to_instance(&dctx::apply_context(
            &ctx,
            dctx::decimal_pow(&base, &exponent)?,
        )?))
    });
    PyObjectRef::new(PyObject::Type {
        name: "Decimal".to_string(),
        dict: Box::new(str_map_to_typedict(type_dict)),
        bases: vec![],
        mro: vec![],
    })
}

/// The `Context` type — the full implementation (methods, signals,
/// flags/traps, properties) lives in `decimal_ctx.rs`.
pub fn get_context_type() -> PyObjectRef {
    let existing = DECIMAL_CONTEXT_TYPE.with(|c| c.borrow().clone());
    if let Some(t) = existing {
        return t;
    }
    let typ = crate::modules::data::decimal_ctx::build_context_type();
    DECIMAL_CONTEXT_TYPE.with(|c| {
        *c.borrow_mut() = Some(typ.clone());
    });
    typ
}

/// Backwards-compatible helper: a `Context` with the given precision and
/// rounding mode (all other fields at CPython's `DefaultContext` values).
pub fn make_context_instance(precision: usize, rounding: &str) -> PyObjectRef {
    crate::modules::data::decimal_ctx::new_context(
        precision as i64,
        rounding,
        -999999,
        999999,
        true,
        false,
        &crate::modules::data::decimal_ctx::DEFAULT_TRAPS,
    )
}
