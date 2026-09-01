use crate::object::*;
use std::collections::HashMap;
use num_bigint::{BigInt, Sign};
use num_traits::{One, Signed, Zero};
use num_traits::ToPrimitive;
use std::rc::Rc;
use crate::modules::data::decimal::*;
use crate::modules::data::fractions::*;
use crate::modules::data::fractions_ops::*;
pub fn create_fractions_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    let mut frac_dict: HashMap<String, PyObjectRef> = HashMap::new();

    // `Fraction.from_float(f)` / `Fraction._from_coprime_ints(n, d)` —
    // classmethods: LOAD_ATTR binds the calling class as args[0].
    frac_dict.insert_str(
        "from_float",
        PyObjectRef::new(PyObject::ClassMethod {
            func: PyObjectRef::imm(PyObject::BuiltinFunction {
                name: "from_float".to_string(),
                func: |args| {
                    if args.len() < 2 {
                        return Err(PyError::type_error("from_float() takes exactly 1 argument"));
                    }
                    let cls = &args[0];
                    let val = &args[1];
                    let vb = val.borrow();
                    if let PyObject::Int(n) = &*vb {
                        // An int argument is just Fraction(int).
                        return frac_make(cls, n.clone(), BigInt::one());
                    }
                    drop(vb);
                    let f = val
                        .as_f64()
                        .ok_or_else(|| PyError::type_error("argument should be a float"))?;
                    let (num, den) = frac_float_to_ratio(f)?;
                    frac_make(cls, num, den)
                },
            }),
        }),
    );
    frac_dict.insert_str(
        "from_number",
        PyObjectRef::new(PyObject::ClassMethod {
            func: PyObjectRef::imm(PyObject::BuiltinFunction {
                name: "from_number".to_string(),
                func: fraction_from_number_fallback,
            }),
        }),
    );
    frac_dict.insert_str(
        "from_decimal",
        PyObjectRef::new(PyObject::ClassMethod {
            func: PyObjectRef::imm(PyObject::BuiltinFunction {
                name: "from_decimal".to_string(),
                func: fraction_from_decimal_fallback,
            }),
        }),
    );
    frac_dict.insert_str(
        "_from_coprime_ints",
        PyObjectRef::new(PyObject::ClassMethod {
            func: PyObjectRef::imm(PyObject::BuiltinFunction {
                name: "_from_coprime_ints".to_string(),
                func: |args| {
                    if args.len() < 3 {
                        return Err(PyError::type_error(
                            "_from_coprime_ints() takes exactly 2 arguments",
                        ));
                    }
                    let cls = &args[0];
                    // Store the raw objects (CPython keeps them as-is) so
                    // `x.numerator` is the actual argument — but validate they are
                    // integers (or indexable / int-subclass instances).
                    let _ = crate::object::int_value_or_backing(&args[1])
                        .or_else(|| crate::object::to_index(&args[1]).ok())
                        .ok_or_else(|| PyError::type_error("numerator must be an integer"))?;
                    let _ = crate::object::int_value_or_backing(&args[2])
                        .or_else(|| crate::object::to_index(&args[2]).ok())
                        .ok_or_else(|| PyError::type_error("denominator must be an integer"))?;
                    let mut dict = AttrMap::new();
                    dict.insert_str("_numerator", args[1].clone());
                    dict.insert_str("_denominator", args[2].clone());
                    Ok(PyObjectRef::new(PyObject::Instance {
                        typ: cls.clone(),
                        dict,
                    }))
                },
            }),
        }),
    );

    // A plain `__init__`, NOT `NATIVE_VALUE_CTOR_KEY` — the latter is only
    // for types whose direct construction returns a raw NATIVE value
    // (`int(5)` returns `PyObject::Int`, never wrapped in an `Instance`;
    // see its own doc comment) and is called with the constructor's real
    // arguments directly, no class prepended. Fraction needs the OPPOSITE:
    // a genuine `PyObject::Instance` (so it participates in ordinary
    // Instance-based arithmetic/comparison dispatch), which is exactly
    // what the standard `__init__` convention already provides — the
    // general Type-call machinery creates a fresh empty `Instance` first,
    // THEN calls `__init__(self, *real_args)` on it, matching a plain
    // `class Fraction: def __init__(self, ...): ...`. (An earlier version
    // of this mistakenly used `NATIVE_VALUE_CTOR_KEY`, which — receiving
    // the raw args directly with no class arg at all — silently
    // misinterpreted the first REAL constructor argument as if it were
    // the class, corrupting every `Fraction(...)` call.)
    frac_dict.insert_str(
        "__init__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__init__".to_string(),
            func: fraction_init_fallback,
        }),
    );

    macro_rules! frac_method {
        ($name:expr, $func:expr) => {
            frac_dict.insert_str(
                $name,
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: $name.to_string(),
                    func: $func,
                }),
            );
        };
    }
    frac_method!("__add__", |args| frac_binop(
        args,
        false,
        |an, ad, bn, bd| Ok((&an * &bd + &bn * &ad, ad * bd)),
        |a, b| a + b,
        |(ar, ai), (br, bi)| (ar + br, ai + bi),
        crate::object::py_add
    ));
    frac_method!("__radd__", |args| frac_binop(
        args,
        true,
        |an, ad, bn, bd| Ok((&an * &bd + &bn * &ad, ad * bd)),
        |a, b| a + b,
        |(ar, ai), (br, bi)| (ar + br, ai + bi),
        crate::object::py_add
    ));
    frac_method!("__sub__", |args| frac_binop(
        args,
        false,
        |an, ad, bn, bd| Ok((&an * &bd - &bn * &ad, ad * bd)),
        |a, b| a - b,
        |(ar, ai), (br, bi)| (ar - br, ai - bi),
        crate::object::py_sub
    ));
    frac_method!("__rsub__", |args| frac_binop(
        args,
        true,
        |an, ad, bn, bd| Ok((&an * &bd - &bn * &ad, ad * bd)),
        |a, b| a - b,
        |(ar, ai), (br, bi)| (ar - br, ai - bi),
        crate::object::py_sub
    ));
    frac_method!("__mul__", |args| frac_binop(
        args,
        false,
        |an, ad, bn, bd| Ok((an * bn, ad * bd)),
        |a, b| a * b,
        |(ar, ai), (br, bi)| (ar * br - ai * bi, ar * bi + ai * br),
        crate::object::py_mul
    ));
    frac_method!("__rmul__", |args| frac_binop(
        args,
        true,
        |an, ad, bn, bd| Ok((an * bn, ad * bd)),
        |a, b| a * b,
        |(ar, ai), (br, bi)| (ar * br - ai * bi, ar * bi + ai * br),
        crate::object::py_mul
    ));
    frac_method!("__truediv__", |args| frac_binop(
        args,
        false,
        |an, ad, bn, bd| {
            if bn.is_zero() {
                return Err(PyError::ZeroDivisionError(
                    "Fraction division by zero".to_string(),
                ));
            }
            Ok((an * bd, ad * bn))
        },
        |a, b| a / b,
        |(ar, ai), (br, bi)| {
            // Smith's algorithm (matching CPython's complex division).
            if br.abs() >= bi.abs() {
                let ratio = bi / br;
                let denom = br + bi * ratio;
                ((ar + ai * ratio) / denom, (ai - ar * ratio) / denom)
            } else {
                let ratio = br / bi;
                let denom = br * ratio + bi;
                ((ar * ratio + ai) / denom, (ai * ratio - ar) / denom)
            }
        },
        crate::object::py_div
    ));
    frac_method!("__rtruediv__", |args| frac_binop(
        args,
        true,
        |an, ad, bn, bd| {
            if bn.is_zero() {
                return Err(PyError::ZeroDivisionError(
                    "Fraction division by zero".to_string(),
                ));
            }
            Ok((an * bd, ad * bn))
        },
        |a, b| a / b,
        |(ar, ai), (br, bi)| {
            // Smith's algorithm (matching CPython's complex division).
            if br.abs() >= bi.abs() {
                let ratio = bi / br;
                let denom = br + bi * ratio;
                ((ar + ai * ratio) / denom, (ai - ar * ratio) / denom)
            } else {
                let ratio = br / bi;
                let denom = br * ratio + bi;
                ((ar * ratio + ai) / denom, (ai * ratio - ar) / denom)
            }
        },
        crate::object::py_div
    ));
    frac_method!("__floordiv__", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("expected 2 arguments"));
        }
        let (an, ad) = frac_self_num_den(&args[0])?;
        match frac_operand_of(&args[1]) {
            FracOperand::Frac(bn, bd) => {
                if !frac_forward_ok(&args[1]) {
                    return Ok(py_not_implemented());
                }
                if bn.is_zero() {
                    return Err(PyError::ZeroDivisionError(
                        "Fraction division by zero".to_string(),
                    ));
                }
                Ok(py_int(floor_div_rem(&an * &bd, &(&ad * &bn)).0))
            }
            FracOperand::Float(bf) => {
                if bf == 0.0 {
                    return Err(PyError::ZeroDivisionError(
                        "float floor division by zero".to_string(),
                    ));
                }
                let af = frac_to_f64(&an, &ad);
                Ok(py_float((af / bf).floor()))
            }
            FracOperand::Other => Ok(py_not_implemented()),
        }
    });
    frac_method!("__rfloordiv__", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("expected 2 arguments"));
        }
        let (bn, bd) = frac_self_num_den(&args[0])?;
        match frac_operand_of(&args[1]) {
            FracOperand::Frac(an, ad) => {
                if an.is_zero() {
                    return Err(PyError::ZeroDivisionError(
                        "Fraction division by zero".to_string(),
                    ));
                }
                Ok(py_int(floor_div_rem(&an * &bd, &(&ad * &bn)).0))
            }
            FracOperand::Float(af) => {
                if af == 0.0 {
                    return Err(PyError::ZeroDivisionError(
                        "float floor division by zero".to_string(),
                    ));
                }
                let bf = frac_to_f64(&bn, &bd);
                Ok(py_float((af / bf).floor()))
            }
            FracOperand::Other => {
                if let Some(r) =
                    frac_reflected_float(&args[1], frac_to_f64(&bn, &bd), |a, b| (a / b).floor())
                {
                    return Ok(r);
                }
                Ok(py_not_implemented())
            }
        }
    });
    frac_method!("__mod__", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("expected 2 arguments"));
        }
        let (an, ad) = frac_self_num_den(&args[0])?;
        match frac_operand_of(&args[1]) {
            FracOperand::Frac(bn, bd) => {
                if !frac_forward_ok(&args[1]) {
                    return Ok(py_not_implemented());
                }
                if bn.is_zero() {
                    return Err(PyError::ZeroDivisionError(
                        "Fraction modulo by zero".to_string(),
                    ));
                }
                let q = floor_div_rem(&an * &bd, &(&ad * &bn)).0;
                let (rn, rd) = frac_normalize(&an * &bd - &bn * &ad * q, &ad * &bd)?;
                frac_make(&get_fraction_type(), rn, rd)
            }
            FracOperand::Float(bf) => crate::object::py_float_mod(frac_to_f64(&an, &ad), bf),
            FracOperand::Other => Ok(py_not_implemented()),
        }
    });
    frac_method!("__rmod__", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("expected 2 arguments"));
        }
        let (bn, bd) = frac_self_num_den(&args[0])?;
        match frac_operand_of(&args[1]) {
            FracOperand::Frac(an, ad) => {
                if an.is_zero() {
                    return Err(PyError::ZeroDivisionError(
                        "Fraction modulo by zero".to_string(),
                    ));
                }
                let q = floor_div_rem(&an * &bd, &(&ad * &bn)).0;
                let (rn, rd) = frac_normalize(&an * &bd - &bn * &ad * q, &ad * &bd)?;
                frac_make(&get_fraction_type(), rn, rd)
            }
            FracOperand::Float(af) => crate::object::py_float_mod(af, frac_to_f64(&bn, &bd)),
            FracOperand::Other => {
                let mod_op = |a: f64, b: f64| -> f64 {
                    let rem = a % b;
                    if rem == 0.0 {
                        if b.is_sign_negative() {
                            -0.0
                        } else {
                            0.0
                        }
                    } else if (rem < 0.0) != (b < 0.0) {
                        rem + b
                    } else {
                        rem
                    }
                };
                if let Some(r) = frac_reflected_float(&args[1], frac_to_f64(&bn, &bd), mod_op) {
                    return Ok(r);
                }
                Ok(py_not_implemented())
            }
        }
    });
    frac_method!("__divmod__", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("expected 2 arguments"));
        }
        let (an, ad) = frac_self_num_den(&args[0])?;
        match frac_operand_of(&args[1]) {
            FracOperand::Frac(bn, bd) => {
                if !frac_forward_ok(&args[1]) {
                    return Ok(py_not_implemented());
                }
                if bn.is_zero() {
                    return Err(PyError::ZeroDivisionError(
                        "Fraction division by zero".to_string(),
                    ));
                }
                let q = floor_div_rem(&an * &bd, &(&ad * &bn)).0;
                let r = frac_normalize(&an * &bd - &bn * &ad * &q, &ad * &bd)?;
                Ok(py_tuple(vec![
                    py_int(q),
                    frac_make(&get_fraction_type(), r.0, r.1)?,
                ]))
            }
            FracOperand::Float(bf) => {
                if bf == 0.0 {
                    return Err(PyError::ZeroDivisionError(
                        "float division by zero".to_string(),
                    ));
                }
                let af = frac_to_f64(&an, &ad);
                let q = (af / bf).floor();
                let r = crate::object::py_float_mod(af, bf)?;
                Ok(py_tuple(vec![py_float(q), r]))
            }
            FracOperand::Other => Ok(py_not_implemented()),
        }
    });
    frac_method!("__rdivmod__", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("expected 2 arguments"));
        }
        let (bn, bd) = frac_self_num_den(&args[0])?;
        match frac_operand_of(&args[1]) {
            FracOperand::Frac(an, ad) => {
                if an.is_zero() {
                    return Err(PyError::ZeroDivisionError(
                        "Fraction division by zero".to_string(),
                    ));
                }
                let q = floor_div_rem(&an * &bd, &(&ad * &bn)).0;
                let r = frac_normalize(&an * &bd - &bn * &ad * &q, &ad * &bd)?;
                Ok(py_tuple(vec![
                    py_int(q),
                    frac_make(&get_fraction_type(), r.0, r.1)?,
                ]))
            }
            FracOperand::Float(af) => {
                if af == 0.0 {
                    return Err(PyError::ZeroDivisionError(
                        "float division by zero".to_string(),
                    ));
                }
                let bf = frac_to_f64(&bn, &bd);
                let q = (af / bf).floor();
                let r = crate::object::py_float_mod(af, bf)?;
                Ok(py_tuple(vec![py_float(q), r]))
            }
            FracOperand::Other => {
                if let Some(other_f) = frac_reflected_float_value(&args[1]) {
                    let bf = frac_to_f64(&bn, &bd);
                    if other_f != 0.0 {
                        let q = (other_f / bf).floor();
                        let rem = crate::object::py_float_mod(other_f, bf).ok();
                        return Ok(py_tuple(vec![
                            py_float(q),
                            rem.unwrap_or_else(|| py_float(f64::NAN)),
                        ]));
                    }
                }
                Ok(py_not_implemented())
            }
        }
    });
    frac_method!("__pow__", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("expected 2 arguments"));
        }
        // 3-argument `pow(base, exp, mod)` is not supported for Fraction.
        if args.len() >= 3 && !matches!(&*args[2].borrow(), PyObject::None) {
            return Ok(py_not_implemented());
        }
        let (an, ad) = frac_self_num_den(&args[0])?;
        match frac_operand_of(&args[1]) {
            FracOperand::Frac(bn, bd) if bd == BigInt::one() => {
                let (rn, rd) = frac_rational_pow(&an, &ad, &bn)?;
                frac_make(&get_fraction_type(), rn, rd)
            }
            FracOperand::Frac(bn, bd) => {
                Ok(frac_float_pow(frac_to_f64(&an, &ad), frac_to_f64(&bn, &bd)))
            }
            FracOperand::Float(bf) => Ok(frac_float_pow(frac_to_f64(&an, &ad), bf)),
            FracOperand::Other => {
                // CPython's `isinstance(b, (float, complex))` arm:
                // `float(a) ** b` (delegates to a complex(-subclass)
                // exponent's own `__rpow__`).
                if frac_is_complex_operand(&args[1]) {
                    return crate::object::py_pow(&py_float(frac_to_f64(&an, &ad)), &args[1]);
                }
                Ok(py_not_implemented())
            }
        }
    });
    frac_method!("__rpow__", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("expected 2 arguments"));
        }
        if args.len() >= 3 && !matches!(&*args[2].borrow(), PyObject::None) {
            return Ok(py_not_implemented());
        }
        // self is the EXPONENT b in `a ** b` (CPython's Fraction.__rpow__).
        let (bn, bd) = frac_self_num_den(&args[0])?;
        let a = &args[1];
        // b integer and >= 0: `a ** b.numerator` keeps an int base an int.
        if bd == BigInt::one() && bn.sign() != num_bigint::Sign::Minus {
            return crate::object::py_pow(a, &py_int(bn.clone()));
        }
        match frac_operand_of(a) {
            FracOperand::Frac(an, ad) => {
                // Rational base -> Fraction(base) ** b (integer b handled
                // exactly; non-integer b -> float).
                if bd == BigInt::one() {
                    let (rn, rd) = frac_rational_pow(&an, &ad, &bn)?;
                    frac_make(&get_fraction_type(), rn, rd)
                } else {
                    Ok(frac_float_pow(frac_to_f64(&an, &ad), frac_to_f64(&bn, &bd)))
                }
            }
            FracOperand::Float(af) => {
                if bd == BigInt::one() {
                    Ok(py_float(af.powi(bn.to_i32().unwrap_or(0))))
                } else {
                    Ok(frac_float_pow(af, frac_to_f64(&bn, &bd)))
                }
            }
            FracOperand::Other => {
                // CPython's `b.denominator == 1` arm: `a ** b.numerator`
                // (for non-Rational bases like a complex subclass, keeping
                // exactness where possible).
                if bd == BigInt::one() {
                    if let Ok(r) = crate::object::py_pow(a, &py_int(bn.clone())) {
                        if !crate::object::is_not_implemented(&r) {
                            return Ok(r);
                        }
                    }
                }
                // CPython's final `a ** float(b)` arm for Real/Complex bases.
                let bf = frac_to_f64(&bn, &bd);
                let f = a.borrow().get_attribute("__pow__").ok();
                if let Some(f) = f {
                    if let Ok(r) =
                        crate::object::call_bound_method(f, a.clone(), vec![py_float(bf)])
                    {
                        if !matches!(&*r.borrow(), PyObject::None) {
                            return Ok(r);
                        }
                    }
                }
                Ok(py_not_implemented())
            }
        }
    });
    frac_method!("__neg__", |args| {
        let (n, d) = frac_self_num_den(&args[0])?;
        frac_make(&get_fraction_type(), -n, d)
    });
    frac_method!("__pos__", |args| {
        let (n, d) = frac_self_num_den(&args[0])?;
        frac_make(&get_fraction_type(), n, d)
    });
    frac_method!("__abs__", |args| {
        let (n, d) = frac_self_num_den(&args[0])?;
        frac_make(&get_fraction_type(), n.abs(), d)
    });
    frac_method!("__float__", |args| {
        let (n, d) = frac_self_num_den(&args[0])?;
        Ok(py_float(frac_to_f64(&n, &d)))
    });
    frac_method!("__complex__", |args| {
        let (n, d) = frac_self_num_den(&args[0])?;
        Ok(PyObjectRef::imm(PyObject::Complex(
            frac_to_f64(&n, &d),
            0.0,
        )))
    });
    frac_method!("__int__", |args| {
        let (n, d) = frac_self_num_den(&args[0])?;
        Ok(py_int(n / d))
    });
    frac_method!("__trunc__", |args| {
        let (n, d) = frac_self_num_den(&args[0])?;
        Ok(py_int(n / d))
    });
    frac_method!("__floor__", |args| {
        let (n, d) = frac_self_num_den(&args[0])?;
        Ok(py_int(floor_div_rem(n, &d).0))
    });
    frac_method!("__ceil__", |args| {
        let (n, d) = frac_self_num_den(&args[0])?;
        let (q, r) = floor_div_rem(n, &d);
        Ok(py_int(if r.is_zero() { q } else { q + 1 }))
    });
    frac_method!("__round__", |args| {
        let (n, d) = frac_self_num_den(&args[0])?;
        let round_int = |num: &BigInt, den: &BigInt| -> BigInt {
            let q = floor_div_rem(num.clone(), den).0;
            let r: BigInt = num - &q * den;
            if &r * 2 < *den {
                q
            } else if &r * 2 > *den {
                q + 1
            } else if (&q % BigInt::from(2)).is_zero() {
                q
            } else {
                q + 1
            }
        };
        if args.len() < 2 || matches!(&*args[1].borrow(), PyObject::None) {
            return Ok(py_int(round_int(&n, &d)));
        }
        let ndigits = args[1].as_i64().ok_or_else(|| {
            PyError::type_error("__round__() argument 'ndigits' must be integral")
        })?;
        let shift = BigInt::from(10).pow(ndigits.unsigned_abs() as u32);
        let (rn, rd) = if ndigits > 0 {
            (round_int(&(n * &shift), &d), shift)
        } else {
            (round_int(&n, &(d * &shift)) * shift, BigInt::one())
        };
        frac_make(&get_fraction_type(), rn, rd)
    });
    frac_method!("limit_denominator", |args| {
        let max_den = if args.len() < 2 || matches!(&*args[1].borrow(), PyObject::None) {
            BigInt::from(1_000_000)
        } else {
            crate::object::int_value_or_backing(&args[1])
                .or_else(|| crate::object::to_index(&args[1]).ok())
                .ok_or_else(|| PyError::type_error("max_denominator should be an integer"))?
        };
        if max_den < BigInt::one() {
            return Err(PyError::value_error("max_denominator should be at least 1"));
        }
        let (n, d) = frac_self_num_den(&args[0])?;
        if d <= max_den {
            return Ok(args[0].clone());
        }
        // Continued-fraction nearest-fraction search (port of CPython's
        // Fraction.limit_denominator).
        let (orig_n, orig_d) = (n.clone(), d.clone());
        let (mut p0, mut q0) = (BigInt::zero(), BigInt::one());
        let (mut p1, mut q1) = (BigInt::one(), BigInt::zero());
        let (mut n, mut d) = (n, d);
        loop {
            let a = &n / &d;
            let q2 = &q0 + &a * &q1;
            if q2 > max_den {
                break;
            }
            let (np0, nq0) = (p1.clone(), q1.clone());
            p1 = &a * &p1 + &p0;
            q1 = q2;
            p0 = np0;
            q0 = nq0;
            let new_n = &n - &a * &d;
            n = d;
            d = new_n;
        }
        let k = (&max_den - &q0) / &q1;
        let b1n = &p0 + &k * &p1;
        let b1d = &q0 + &k * &q1;
        // Pick whichever candidate is closer to self (ties -> smaller
        // denominator, i.e. bound2), comparing cross-multiplied distances.
        let diff2 = (&p1 * &orig_d - &q1 * &orig_n).abs() * &b1d;
        let diff1 = (&b1n * &orig_d - &b1d * &orig_n).abs() * &q1;
        let (rn, rd) = if diff2 <= diff1 { (p1, q1) } else { (b1n, b1d) };
        frac_make(&get_fraction_type(), rn, rd)
    });
    frac_method!("__bool__", |args| {
        // CPython uses `bool(self._numerator)` — a raw (int-subclass /
        // registered-Rational) numerator's own `__bool__` is consulted.
        if let PyObject::Instance { dict, .. } = &*args[0].borrow() {
            if let Some(num) = dict.get_str("_numerator") {
                return Ok(py_bool(num.truthy()));
            }
        }
        let (n, _d) = frac_self_num_den(&args[0])?;
        Ok(py_bool(!n.is_zero()))
    });
    frac_method!("__repr__", |args| {
        let (n, d) = frac_self_num_den(&args[0])?;
        Ok(py_str(&format!("Fraction({}, {})", n, d)))
    });
    frac_method!("__str__", |args| {
        let (n, d) = frac_self_num_den(&args[0])?;
        if d == BigInt::one() {
            Ok(py_str(&n.to_string()))
        } else {
            Ok(py_str(&format!("{}/{}", n, d)))
        }
    });
    frac_method!("__format__", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("__format__ requires 1 argument"));
        }
        if matches!(&*args[1].borrow(), PyObject::None) {
            return Err(PyError::type_error(
                "format() argument 2 must be str, not NoneType",
            ));
        }
        let spec = args[1].str();
        if spec.is_empty() {
            return Ok(py_str(&args[0].str()));
        }
        let (n, d) = frac_self_num_den(&args[0])?;
        let d = if d == BigInt::zero() {
            BigInt::one()
        } else {
            d
        };
        // Specs without a float presentation type use CPython's
        // _format_general (sign/alt/thousands-sep over the str() form);
        // everything else is float-style. Try general first, like CPython.
        let out = match frac_parse_general_spec(&spec) {
            Some(parts) => frac_format_general(n, &d, &parts)?,
            None => frac_format_exact(n, d, &spec)?,
        };
        Ok(py_str(&out))
    });
    frac_method!("__hash__", |args| {
        let (n, d) = frac_self_num_den(&args[0])?;
        // CPython's _hash_algorithm: hash(|N| * dinv) mod 2**61-1, with INF
        // (314159) when the denominator has no modular inverse (is a multiple
        // of the modulus), signed by the numerator.
        let modulus = (BigInt::from(1i64) << 61) - BigInt::from(1);
        let hash_ = match crate::object::bigint_mod_inverse(&d, &modulus) {
            None => 314159i64, // _PyHASH_INF
            Some(inv) => {
                let abs_n_hash = crate::object::hash_bigint(&n.abs());
                let product = BigInt::from(abs_n_hash as i64) * inv;
                crate::object::hash_bigint(&product) as i64
            }
        };
        let result = if n.sign() == num_bigint::Sign::Minus {
            -hash_
        } else {
            hash_
        };
        Ok(py_int(if result == -1 { -2 } else { result }))
    });
    frac_method!("__eq__", |args| {
        if args.len() < 2 {
            return Ok(py_bool(false));
        }
        let (an, ad) = frac_self_num_den(&args[0])?;
        match frac_operand_of(&args[1]) {
            FracOperand::Frac(bn, bd) => Ok(py_bool(an == bn && ad == bd)),
            FracOperand::Float(bf) => Ok(py_bool(match frac_cmp_exact(&an, &ad, bf) {
                Some(o) => o.is_eq(),
                None => false,
            })),
            FracOperand::Other => {
                // CPython: `isinstance(b, Complex) and b.imag == 0` ->
                // compare against the real part as a float (exactly).
                let complex_val: Option<(f64, f64)> =
                    if let PyObject::Complex(re, im) = &*args[1].borrow() {
                        Some((*re, *im))
                    } else if frac_is_complex_operand(&args[1]) {
                        match args[1].borrow().get_attribute("__complex__") {
                            Ok(f) => crate::object::call_bound_method(f, args[1].clone(), vec![])
                                .ok()
                                .and_then(|c| {
                                    let cb = c.borrow();
                                    if let PyObject::Complex(re, im) = &*cb {
                                        Some((*re, *im))
                                    } else {
                                        None
                                    }
                                }),
                            Err(_) => None,
                        }
                    } else {
                        None
                    };
                if let Some((re, im)) = complex_val {
                    if im == 0.0 {
                        return Ok(py_bool(match frac_cmp_exact(&an, &ad, re) {
                            Some(o) => o.is_eq(),
                            None => false,
                        }));
                    }
                }
                Ok(py_not_implemented())
            }
        }
    });
    macro_rules! frac_cmp {
        ($name:expr, $cmp:expr) => {
            frac_method!($name, |args| {
                if args.len() < 2 {
                    return Ok(py_not_implemented());
                }
                let (an, ad) = frac_self_num_den(&args[0])?;
                match frac_operand_of(&args[1]) {
                    FracOperand::Frac(bn, bd) => Ok(py_bool($cmp((an * &bd).cmp(&(bn * &ad))))),
                    FracOperand::Float(bf) => {
                        match frac_cmp_exact(&an, &ad, bf) {
                            Some(o) => Ok(py_bool($cmp(o))),
                            // NaN involved: every ordered comparison is False.
                            None => Ok(py_bool(false)),
                        }
                    }
                    FracOperand::Other => Ok(py_not_implemented()),
                }
            });
        };
    }
    frac_cmp!("__lt__", |o: std::cmp::Ordering| o.is_lt());
    frac_cmp!("__le__", |o: std::cmp::Ordering| o.is_le());
    frac_cmp!("__gt__", |o: std::cmp::Ordering| o.is_gt());
    frac_cmp!("__ge__", |o: std::cmp::Ordering| o.is_ge());
    frac_method!("as_integer_ratio", |args| {
        let (n, d) = frac_self_num_den(&args[0])?;
        Ok(py_tuple(vec![py_int(n), py_int(d)]))
    });
    // Read-only `numerator`/`denominator` properties backed by the
    // `_numerator`/`_denominator` slots — the raw stored objects are
    // returned (an int-subclass or registered-Rational `numerator` from the
    // constructor is preserved, matching CPython).
    frac_dict.insert_str(
        "numerator",
        PyObjectRef::new(PyObject::Property(Box::new(PropertyData {
            getter: Some(PyObjectRef::new(PyObject::BuiltinFunction {
                name: "numerator".to_string(),
                func: |args| {
                    if let PyObject::Instance { dict, .. } = &*args[0].borrow() {
                        if let Some(v) = dict.get_str("_numerator") {
                            return Ok(v.clone());
                        }
                    }
                    Err(PyError::runtime_error("fraction has no _numerator"))
                },
            })),
            setter: None,
            deleter: None,
            doc: None,
        }))),
    );
    frac_dict.insert_str(
        "denominator",
        PyObjectRef::new(PyObject::Property(Box::new(PropertyData {
            getter: Some(PyObjectRef::new(PyObject::BuiltinFunction {
                name: "denominator".to_string(),
                func: |args| {
                    if let PyObject::Instance { dict, .. } = &*args[0].borrow() {
                        if let Some(v) = dict.get_str("_denominator") {
                            return Ok(v.clone());
                        }
                    }
                    Err(PyError::runtime_error("fraction has no _denominator"))
                },
            })),
            setter: None,
            deleter: None,
            doc: None,
        }))),
    );
    // Slots semantics: only `_numerator`/`_denominator` (and the usual
    // instance internals) may be assigned; anything else raises
    // AttributeError (CPython's `Fraction.__slots__`).
    frac_dict.insert_str(
        "__slots__",
        py_tuple(vec![py_str("_numerator"), py_str("_denominator")]),
    );
    frac_method!("is_integer", |args| {
        let (_, d) = frac_self_num_den(&args[0])?;
        Ok(py_bool(d == BigInt::one()))
    });
    frac_method!("__reduce__", |args| {
        let (n, d) = frac_self_num_den(&args[0])?;
        Ok(py_tuple(vec![
            get_fraction_type(),
            py_tuple(vec![py_int(n), py_int(d)]),
        ]))
    });
    frac_method!("__copy__", |args| {
        let (n, d) = frac_self_num_den(&args[0])?;
        if let PyObject::Instance { typ, .. } = &*args[0].borrow() {
            if typ.is(&get_fraction_type()) {
                // Immutable: I am my own clone.
                return Ok(args[0].clone());
            }
            return frac_make(typ, n, d);
        }
        Ok(args[0].clone())
    });
    frac_method!("__deepcopy__", |args| {
        let (n, d) = frac_self_num_den(&args[0])?;
        if let PyObject::Instance { typ, .. } = &*args[0].borrow() {
            if typ.is(&get_fraction_type()) {
                return Ok(args[0].clone());
            }
            return frac_make(typ, n, d);
        }
        Ok(args[0].clone())
    });

    let frac_type = PyObjectRef::new(PyObject::Type {
        name: "Fraction".to_string(),
        dict: Box::new(str_map_to_typedict(frac_dict)),
        bases: vec![],
        mro: vec![],
    });
    // Register for `type.__subclasses__` / pickle's class lookup, with the
    // `__module__` attribute so `pickle.dumps(Fraction(...))` can resolve it.
    if let PyObject::Type { dict, .. } = &mut *frac_type.borrow_mut() {
        dict.insert_str("__module__", py_str("fractions"));
    }
    crate::object::register_class(&frac_type);
    FRACTION_TYPE.with(|c| {
        *c.borrow_mut() = Some(frac_type.clone());
    });
    d.insert_str("Fraction", frac_type);
    d
}

