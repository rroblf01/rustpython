// Split out of the former monolithic object/builtins.rs — this file holds
// numeric builtins (`divmod`, `round`, `abs`, `pow`) and their helpers
// (`f64_exact_ratio`, `round_half_even_rat`, `double_round`, bigint helpers).
use super::*;

pub fn builtin_divmod(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 2 {
        return Err(PyError::type_error("divmod() takes exactly 2 arguments"));
    }
    // Was: unconditional `args[0].as_i64()`/`args[1].as_i64()` — never
    // consulted `__divmod__`/`__rdivmod__` at all, so ANY custom numeric
    // type (real trigger: `numbers.Real`'s own MIXIN `__divmod__`/
    // `__rdivmod__`, already implemented in `Lib/numbers.py` in terms of
    // `__floordiv__`/`__mod__` — exercised directly by CPython's own
    // `test_abstract_numbers.py::test_real`) raised `TypeError: divmod()
    // arg must be int` instead of dispatching to it. Also silently
    // rejected plain `float` arguments, which real `divmod()` supports
    // natively. Mirrors the established `try_dunder_binop` dispatch
    // pattern already used by `py_add`/etc.
    if let Some(r) = try_dunder_binop(&args[0], &args[1], "__divmod__")? {
        return Ok(r);
    }
    if let Some(r) = try_dunder_binop(&args[1], &args[0], "__rdivmod__")? {
        return Ok(r);
    }
    // Python's `//`/`%` floor toward negative infinity, unlike Rust's
    // truncating `/`/`%` — reuse the already-correct `py_floordiv`/`py_mod`
    // (which already raise `ZeroDivisionError` themselves) rather than
    // duplicating that sign-handling logic here.
    let q = match py_floor_div(&args[0], &args[1]) {
        Err(e) => return Err(translate_divmod_error(e, &args[0], &args[1])),
        Ok(q) => q,
    };
    let r = match py_mod(&args[0], &args[1]) {
        Err(e) => return Err(translate_divmod_error(e, &args[0], &args[1])),
        Ok(r) => r,
    };
    Ok(PyObjectRef::new(PyObject::Tuple(vec![q, r])))
}

/// `divmod(a, b)` reports an unsupported-operand failure with a `divmod()`
/// wording rather than the `//`/`%` one CPython's own operators use.
fn translate_divmod_error(e: PyError, a: &PyObjectRef, b: &PyObjectRef) -> PyError {
    match &e {
        PyError::TypeError(msg)
            if msg.starts_with("unsupported operand type(s) for //:")
                || msg.starts_with("unsupported operand type(s) for %:") =>
        {
            let dn = |o: &PyObjectRef| -> String {
                match &*o.borrow() {
                    PyObject::Instance { typ, .. } => {
                        crate::object::get_type_name_for_instance(typ)
                    }
                    o => o.type_name(),
                }
            };
            PyError::type_error(format!(
                "unsupported operand type(s) for divmod(): '{}' and '{}'",
                dn(a),
                dn(b)
            ))
        }
        _ => e,
    }
}

// Exact rational form of an `f64` (as `m * 2^e`), so that `round()` can do
// correctly-rounded decimal arithmetic with `BigInt` instead of the naive
// multiply-then-round that CPython explicitly avoids (double-rounding error,
// broken half-even ties, overflowing intermediates).
fn f64_exact_ratio(x: f64) -> (BigInt, BigInt) {
    let bits = x.to_bits();
    let sign = if (bits >> 63) == 0 { 1i64 } else { -1i64 };
    let biased_exp = ((bits >> 52) & 0x7ff) as i64;
    let mantissa = (bits & 0x000f_ffff_ffff_ffff) as i64;
    if biased_exp == 0 {
        // subnormal: mantissa * 2^-1074
        (BigInt::from(mantissa * sign), BigInt::from(1i64) << 1074)
    } else {
        let full = 0x0010_0000_0000_0000 | mantissa;
        let exp = biased_exp - 1023 - 52;
        if exp >= 0 {
            (BigInt::from(full * sign) << exp, BigInt::from(1i64))
        } else {
            (BigInt::from(full * sign), BigInt::from(1i64) << (-exp))
        }
    }
}

// Round the rational `num/den` (den > 0) to the nearest integer, ties to
// even — what CPython's `_Py_dg_dtoa(x, 3, 0)`-style rounding does exactly.
pub(crate) fn round_half_even_rat(num: &BigInt, den: &BigInt) -> BigInt {
    let neg = num.sign() == num_bigint::Sign::Minus;
    let a = num.abs();
    let q = &a / den;
    let r = &a % den;
    let two_r = &r * 2;
    let q = if &two_r > den {
        q + 1
    } else if &two_r < den {
        q
    } else if (&q % 2) == BigInt::zero() {
        q
    } else {
        q + 1
    };
    if neg {
        -q
    } else {
        q
    }
}

// CPython's `double_round`: round `x` to `ndigits` decimal places with
// correct rounding on the EXACT value of `x`, then return the double nearest
// to that decimal (the "build a decimal string and strtod it back" trick,
// so ties re-round to the nearest double without double-rounding error).
fn double_round(x: f64, ndigits: i64) -> PyResult<f64> {
    let (num, den) = f64_exact_ratio(x);
    let pow10 = BigInt::from(10u32).pow(ndigits.unsigned_abs() as u32);
    let neg = num.sign() == num_bigint::Sign::Minus;
    let (snum, sden) = if ndigits >= 0 {
        (num * &pow10, den)
    } else {
        (num, den * &pow10)
    };
    let q = round_half_even_rat(&snum, &sden);
    // Reconstruct `q / 10^ndigits` as a decimal string.
    let digits = {
        let qs = q.abs().to_string();
        if ndigits >= 0 {
            let n = ndigits as usize;
            if qs.len() > n {
                let (a, b) = qs.split_at(qs.len() - n);
                format!("{}.{}", a, b)
            } else {
                format!("0.{}{}", "0".repeat(n - qs.len()), qs)
            }
        } else {
            format!("{}{}", qs, "0".repeat((-ndigits) as usize))
        }
    };
    // The sign comes from the exact ratio so `round(-0.5, -308)` stays `-0.0`.
    let sign = if neg { "-" } else { "" };
    let rounded: f64 = format!("{}{}", sign, digits).parse().unwrap_or(0.0);
    if rounded.is_infinite() {
        return Err(PyError::overflow_error(
            "rounded value too large to represent",
        ));
    }
    Ok(rounded)
}

pub fn builtin_round(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() > 2 {
        return Err(PyError::type_error("round() takes at most 2 arguments"));
    }
    if args.is_empty() {
        return Err(PyError::type_error(
            "round() missing required argument 'number' (pos 1)",
        ));
    }
    // `round(number=x, ndigits=n)` packs BOTH keywords into a leading dict;
    // `round(x, ndigits=n)` leaves `x` positional and packs only `ndigits`.
    let (val, ndigits_obj): (Option<PyObjectRef>, Option<PyObjectRef>) = {
        if let PyObject::Dict(pd) = &*args[0].borrow() {
            (
                pd.get(&py_str("number")).ok().flatten(),
                pd.get(&py_str("ndigits")).ok().flatten(),
            )
        } else {
            let v = Some(args[0].clone());
            let n = if args.len() > 1 {
                if let PyObject::Dict(pd) = &*args[1].borrow() {
                    pd.get(&py_str("ndigits")).ok().flatten()
                } else {
                    Some(args[1].clone())
                }
            } else {
                None
            };
            (v, n)
        }
    };
    let val = val
        .ok_or_else(|| PyError::type_error("round() missing required argument 'number' (pos 1)"))?;
    // `round(x, None)` / `round(x, ndigits=None)` — an EXPLICIT `None` for
    // `ndigits` behaves exactly like the 1-arg form.
    let has_ndigits = ndigits_obj
        .as_ref()
        .map(|n| !matches!(&*n.borrow(), PyObject::None))
        .unwrap_or(false);

    // Non-numeric objects delegate to `type(x).__round__`, like CPython.
    let is_numeric = matches!(
        &*val.borrow(),
        PyObject::Int(_) | PyObject::Bool(_) | PyObject::Float(_)
    );
    if !is_numeric {
        let f = {
            let o = val.borrow();
            match &*o {
                PyObject::Instance { typ, .. } => lookup_dunder_via_mro(typ, "__round__"),
                _ => None,
            }
        };
        return match f {
            Some(f) => call_bound_method(
                f,
                val,
                if has_ndigits {
                    vec![ndigits_obj.unwrap()]
                } else {
                    vec![]
                },
            ),
            None => Err(PyError::type_error(format!(
                "type {} doesn't define __round__ method",
                val.get_type_name()
            ))),
        };
    }

    // Int path: no ndigits (or ndigits >= 0) → identity; ndigits < 0 →
    // round-half-even to the nearest power-of-ten multiple, never via f64.
    if matches!(&*val.borrow(), PyObject::Int(_) | PyObject::Bool(_)) {
        if !has_ndigits {
            return Ok(val);
        }
        let n = to_index(&ndigits_obj.as_ref().unwrap())?;
        if n >= BigInt::zero() {
            return Ok(val);
        }
        let k: u32 = (-&n).to_u64().unwrap_or(u64::MAX).min(u32::MAX as u64) as u32;
        let pow10 = BigInt::from(10u32).pow(k);
        let int = {
            let v = val.borrow();
            match &*v {
                PyObject::Int(i) => i.clone(),
                PyObject::Bool(b) => BigInt::from(*b as i64),
                _ => unreachable!(),
            }
        };
        let q = round_half_even_rat(&int, &pow10);
        return Ok(py_int(q * pow10));
    }

    // Float path.
    let x = match &*val.borrow() {
        PyObject::Float(f) => *f,
        _ => unreachable!(),
    };
    if !has_ndigits {
        // round(x) → nearest integer (ties to even), returned as an int.
        if x.is_infinite() {
            return Err(PyError::overflow_error(
                "cannot convert float infinity to integer",
            ));
        }
        if x.is_nan() {
            return Err(PyError::value_error("cannot convert float NaN to integer"));
        }
        let (num, den) = f64_exact_ratio(x);
        return Ok(py_int(round_half_even_rat(&num, &den)));
    }
    // round(x, ndigits) → float, with CPython's NDIGITS_MAX/MIN clamps.
    let nd = to_index(&ndigits_obj.as_ref().unwrap())?;
    if !x.is_finite() {
        return Ok(py_float(x));
    }
    if nd > BigInt::from(323) {
        return Ok(py_float(x));
    }
    if nd < BigInt::from(-308) {
        return Ok(py_float(0.0 * x));
    }
    let n = nd.to_i64().unwrap_or(0);
    Ok(py_float(double_round(x, n)?))
}

pub fn builtin_abs(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 1 {
        return Err(PyError::type_error("abs() takes exactly one argument"));
    }
    let obj = args[0].borrow();
    match &*obj {
        PyObject::Int(i) => Ok(py_int(i.clone().abs())),
        PyObject::Float(f) => Ok(py_float(f.abs())),
        PyObject::Complex(re, im) => Ok(py_float(re.hypot(*im))),
        PyObject::Bool(b) => Ok(py_int(if *b { 1 } else { 0 })),
        PyObject::Instance { typ, .. } => match lookup_dunder_via_mro(typ, "__abs__") {
            Some(f) => {
                drop(obj);
                call_bound_method(f, args[0].clone(), vec![])
            }
            None => Err(PyError::type_error(format!(
                "bad operand type for abs(): '{}'",
                get_type_name_for_instance(typ)
            ))),
        },
        _ => Err(PyError::type_error(format!(
            "bad operand type for abs(): '{}'",
            obj.type_name()
        ))),
    }
}

fn bigint_mod_python(a: &BigInt, m: &BigInt) -> BigInt {
    let r = a % m;
    if !r.is_zero() && (r.sign() != m.sign()) {
        r + m
    } else {
        r
    }
}

// Plain Euclidean `gcd` — `num-bigint`'s `Integer` trait (which would give
// this for free, along with `extended_gcd`) isn't an explicit dependency of
// this project (only pulled in transitively), so this is hand-rolled rather
// than adding a new direct dependency for one small, standard algorithm.
pub(crate) fn bigint_gcd(a: &BigInt, b: &BigInt) -> BigInt {
    let (mut a, mut b) = (a.abs(), b.abs());
    while !b.is_zero() {
        let t = &a % &b;
        a = b;
        b = t;
    }
    a
}

// Modular inverse via the extended Euclidean algorithm — `None` if `a` and
// `m` aren't coprime (no inverse exists). Result's sign matches `m`'s,
// matching real CPython's own documented `pow(a, -1, m)` behavior ("an
// inverse, with the same sign as m").
pub(crate) fn bigint_mod_inverse(a: &BigInt, m: &BigInt) -> Option<BigInt> {
    let m_abs = m.abs();
    if m_abs.is_zero() {
        return None;
    }
    let (mut old_r, mut r) = (bigint_mod_python(a, &m_abs), m_abs.clone());
    let (mut old_s, mut s) = (BigInt::one(), BigInt::zero());
    while !r.is_zero() {
        let q = &old_r / &r;
        let new_r = &old_r - &q * &r;
        old_r = r;
        r = new_r;
        let new_s = &old_s - &q * &s;
        old_s = s;
        s = new_s;
    }
    if old_r != BigInt::one() {
        return None;
    }
    Some(bigint_mod_python(&old_s, m))
}

pub fn builtin_pow(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Err(PyError::type_error("pow() requires at least 2 arguments"));
    }
    if args.len() == 3 && !matches!(&*args[2].borrow(), PyObject::None) {
        // Real 3-argument `pow(base, exp, mod)` — the previous body computed
        // `py_pow(base, exp)` (a FULL, unreduced power — e.g. literally
        // `50**1001` as a giant bigint) and THEN took it mod `m`, instead of
        // real modular exponentiation (reducing mod `m` at every squaring
        // step, and computing a genuine modular INVERSE for negative
        // exponents rather than `py_pow`'s float fallback for `exp < 0`,
        // which is simply the wrong value entirely). Confirmed via
        // `test_pow.py::test_negative_exponent`: a 100x100 sweep of
        // `pow(a, -1001, m)`-shaped calls, timing out (the giant-bigint
        // path) AND producing wrong results (the float-for-negative-exponent
        // path) simultaneously.
        // 3-arg pow with a complex argument raises "complex modulo" (from
        // complex.__pow__'s mod path), NOT the int-only TypeError.
        if args[..3]
            .iter()
            .any(|a| matches!(&*a.borrow(), PyObject::Complex(..)))
        {
            return Err(PyError::value_error("complex modulo"));
        }
        // Non-integer (e.g. Fraction/float) args first get a chance via
        // `__pow__(exp, mod)` / `__rpow__(base, mod)`; Fraction returns
        // NotImplemented and CPython raises "unsupported operand type(s)
        // for ** or pow()" rather than the int-only message. (Ints are
        // excluded so the all-int fast path below is untouched.)
        let non_int_base = !matches!(&*args[0].borrow(), PyObject::Int(_) | PyObject::Bool(_));
        let non_int_exp = !matches!(&*args[1].borrow(), PyObject::Int(_) | PyObject::Bool(_));
        let non_int_mod = !matches!(&*args[2].borrow(), PyObject::Int(_) | PyObject::Bool(_));
        let is_class_operand = matches!(&*args[0].borrow(), PyObject::Instance { .. })
            || matches!(&*args[1].borrow(), PyObject::Instance { .. })
            || matches!(&*args[2].borrow(), PyObject::Instance { .. });
        if (non_int_base || non_int_exp || non_int_mod) && is_class_operand {
            if let Some(r) = try_dunder_ternop(&args[0], &args[1], &args[2], "__pow__")? {
                return Ok(r);
            }
            if let Some(r) = try_dunder_ternop(&args[1], &args[0], &args[2], "__rpow__")? {
                return Ok(r);
            }
            // A class (Fraction, Decimal, ...) that *implements* __pow__ but
            // refuses the modulus gets the "unsupported operand type(s)"
            // message with all three types; builtin scalar types get the
            // int-only message instead.
            let tn = |i: usize| -> String {
                match &*args[i].borrow() {
                    PyObject::Instance { typ, .. } => {
                        crate::object::get_type_name_for_instance(typ)
                    }
                    o => o.type_name(),
                }
            };
            return Err(PyError::type_error(format!(
                "unsupported operand type(s) for ** or pow(): '{}', '{}', '{}'",
                tn(0),
                tn(1),
                tn(2)
            )));
        }
        // Non-integer (e.g. float) args raise TypeError; CPython: pow(1.5,
        // 2, 3) -> "pow() 3rd argument not allowed unless all arguments are
        // integers".
        let int_err = || {
            PyError::type_error("pow() 3rd argument not allowed unless all arguments are integers")
        };
        let a = to_index(&args[0]).map_err(|_| int_err())?;
        let b = to_index(&args[1]).map_err(|_| int_err())?;
        let m = to_index(&args[2]).map_err(|_| int_err())?;
        if m.is_zero() {
            return Err(PyError::value_error("pow() 3rd argument cannot be 0"));
        }
        let m_abs = m.abs();
        if m_abs.is_one() {
            return Ok(py_int(BigInt::zero()));
        }
        if b.sign() == Sign::Minus {
            if bigint_gcd(&a, &m_abs) != BigInt::one() {
                return Err(PyError::value_error(
                    "base is not invertible for the given modulus",
                ));
            }
            let inv = bigint_mod_inverse(&a, &m).ok_or_else(|| {
                PyError::value_error("base is not invertible for the given modulus")
            })?;
            let exp_abs = (-&b)
                .to_biguint()
                .ok_or_else(|| PyError::value_error("pow() exponent too large"))?;
            let result = bigint_mod_python(&inv, &m_abs).modpow(&BigInt::from(exp_abs), &m_abs);
            return Ok(py_int(bigint_mod_python(&result, &m)));
        }
        let result = bigint_mod_python(&a, &m_abs).modpow(&b, &m_abs);
        return Ok(py_int(bigint_mod_python(&result, &m)));
    }
    let result = py_pow(&args[0], &args[1])?;
    if args.len() == 3 {
        py_mod(&result, &args[2])
    } else {
        Ok(result)
    }
}
