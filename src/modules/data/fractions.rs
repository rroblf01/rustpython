use crate::object::*;
use std::collections::HashMap;
use num_bigint::{BigInt, Sign};
use num_traits::{One, Signed, Zero};
use num_traits::ToPrimitive;
use std::rc::Rc;
use crate::modules::data::decimal::*;
// ---------------------------------------------------------------------------
// fractions.Fraction — a real rational-number type, replacing a former
// complete stub whose constructor just returned a formatted `"num/den"`
// STRING (`py_str`) instead of a genuine Fraction object at all — no
// arithmetic, no `__float__`, no comparisons, nothing beyond what a plain
// string happens to support by coincidence. Found via CPython's own
// `test_math.py::testHypot`, whose `hypot(Fraction(12, 32), Fraction(5,
// 32))` reached `float(a_fraction_shaped_string)` and got `ValueError:
// could not convert string to float: '3/8'`. Represented as a real
// `PyObject::Instance` (native-Type-backed, matching how other ad-hoc
// native classes in this codebase — e.g. `HTTPConnection` — are built) with
// `numerator`/`denominator` stored as plain instance attributes (arbitrary-
// precision `int`s, always reduced to lowest terms with a positive
// denominator), so it participates in the EXISTING Instance-based
// arithmetic/comparison dispatch (`try_dunder_binop`/`try_rich_compare`)
// with no changes needed to `ops_binary.rs`/`ops_compare.rs` at all.
// ---------------------------------------------------------------------------


pub fn frac_bigint_gcd(a: &BigInt, b: &BigInt) -> BigInt {
    let mut a = a.abs();
    let mut b = b.abs();
    while !b.is_zero() {
        let t = b.clone();
        b = &a % &b;
        a = t;
    }
    a
}

pub fn frac_normalize(mut num: BigInt, mut den: BigInt) -> PyResult<(BigInt, BigInt)> {
    if den.is_zero() {
        return Err(PyError::ZeroDivisionError(format!("Fraction({}, 0)", num)));
    }
    if den.sign() == Sign::Minus {
        num = -num;
        den = -den;
    }
    let g = frac_bigint_gcd(&num, &den);
    if g > BigInt::one() {
        num /= &g;
        den /= &g;
    }
    Ok((num, den))
}

/// Exact binary-fraction decomposition of an `f64` (no precision loss) —
/// matches real Python's `float.as_integer_ratio()` / `Fraction.from_float`.
pub fn frac_float_to_ratio(f: f64) -> PyResult<(BigInt, BigInt)> {
    if f.is_nan() {
        return Err(PyError::value_error("cannot convert NaN to integer ratio"));
    }
    if f.is_infinite() {
        return Err(PyError::overflow_error(
            "cannot convert Infinity to integer ratio",
        ));
    }
    if f == 0.0 {
        return Ok((BigInt::zero(), BigInt::one()));
    }
    let bits = f.to_bits();
    let neg = bits >> 63 == 1;
    let biased_exp = ((bits >> 52) & 0x7ff) as i64;
    let mantissa_bits = bits & 0x000f_ffff_ffff_ffff;
    let (mantissa, exp): (u64, i64) = if biased_exp == 0 {
        (mantissa_bits, -1074)
    } else {
        (mantissa_bits | (1u64 << 52), biased_exp - 1075)
    };
    let mut num = BigInt::from(mantissa);
    if neg {
        num = -num;
    }
    let mut den = BigInt::one();
    if exp >= 0 {
        num *= BigInt::from(2).pow(exp as u32);
    } else {
        den = BigInt::from(2).pow((-exp) as u32);
    }
    frac_normalize(num, den)
}

/// Validate a digit group: non-empty, digits only with single underscores
/// strictly BETWEEN digits (`\d+(_\d+)*`), so `_1`, `1_`, `1__2` fail.
pub fn frac_valid_digits(s: &str) -> bool {
    let bytes: Vec<char> = s.chars().collect();
    if bytes.is_empty() || !bytes[0].is_ascii_digit() || !bytes[bytes.len() - 1].is_ascii_digit() {
        return false;
    }
    let mut prev_digit = true;
    for &c in &bytes {
        if c == '_' {
            if !prev_digit {
                return false;
            }
            prev_digit = false;
        } else if c.is_ascii_digit() {
            prev_digit = true;
        } else {
            return false;
        }
    }
    true
}

/// Parse `"3/4"`, `"3"`, `"1.5"`, `"-1.5e2"` (real `Fraction(str)` accepts
/// decimal-literal-like strings too, converting exactly via `from_decimal`
/// semantics) — strict about signs/underscores/whitespace, matching
/// CPython's `_RATIONAL_FORMAT`.
pub fn frac_parse_str(s: &str) -> PyResult<(BigInt, BigInt)> {
    let s = s.trim();
    let bad = || PyError::value_error(format!("Invalid literal for Fraction: '{}'", s));
    let (neg, rest) = if let Some(r) = s.strip_prefix('-') {
        (true, r)
    } else if let Some(r) = s.strip_prefix('+') {
        (false, r)
    } else {
        (false, s)
    };
    let starts_with_digit = rest.starts_with(|c: char| c.is_ascii_digit());
    let starts_with_dot_digit =
        rest.starts_with('.') && rest.len() > 1 && rest.as_bytes()[1].is_ascii_digit();
    if !starts_with_digit && !starts_with_dot_digit {
        return Err(bad());
    }
    // `num / denom` — neither side may carry a sign.
    if rest.contains('/') {
        let mut parts = rest.split('/');
        let n = parts.next().unwrap_or("").trim();
        let d = parts.next().unwrap_or("").trim();
        if parts.next().is_some() || !frac_valid_digits(n) || !frac_valid_digits(d) {
            return Err(bad());
        }
        crate::object::check_int_str_digit_limit(n, None)?;
        crate::object::check_int_str_digit_limit(d, None)?;
        let num: BigInt = n.replace('_', "").parse().map_err(|_| bad())?;
        let den: BigInt = d.replace('_', "").parse().map_err(|_| bad())?;
        return frac_normalize(if neg { -num } else { num }, den);
    }
    // Decimal/exponent form: `num[.frac][e|E[sign]exp]`.
    let (mantissa, exp10) = match rest.find(['e', 'E']) {
        Some(pos) => {
            let exp_str = &rest[pos + 1..];
            let exp_neg = exp_str.starts_with('-');
            let exp_clean = exp_str.strip_prefix(['-', '+']).unwrap_or(exp_str);
            if !frac_valid_digits(exp_clean) {
                return Err(bad());
            }
            crate::object::check_int_str_digit_limit(exp_clean, None)?;
            let exp_val: i64 = exp_clean.replace('_', "").parse().map_err(|_| bad())?;
            (&rest[..pos], if exp_neg { -exp_val } else { exp_val })
        }
        None => (rest, 0),
    };
    let (int_part, frac_part) = match mantissa.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (mantissa, None),
    };
    match frac_part {
        Some(f) => {
            // `3.`, `.5` allowed; `3..5` etc. are caught because the second
            // dot isn't a digit (frac_valid_digits rejects non-digits).
            if !int_part.is_empty() && !frac_valid_digits(int_part) {
                return Err(bad());
            }
            if !f.is_empty() && !frac_valid_digits(f) {
                return Err(bad());
            }
            if int_part.is_empty() && f.is_empty() {
                return Err(bad());
            }
            crate::object::check_int_str_digit_limit(int_part, None)?;
            crate::object::check_int_str_digit_limit(f, None)?;
        }
        None => {
            if !frac_valid_digits(int_part) {
                return Err(bad());
            }
            crate::object::check_int_str_digit_limit(int_part, None)?;
        }
    }
    let int_digits = if int_part.is_empty() { "0" } else { int_part };
    let frac_clean = frac_part.unwrap_or("").replace('_', "");
    let frac_len = frac_clean.len();
    let combined = format!("{}{}", int_digits.replace('_', ""), frac_clean);
    let num_digits: BigInt = combined.parse().map_err(|_| bad())?;
    let scale = -(frac_len as i64);
    let neg = neg || mantissa.starts_with('-');
    let total_exp = scale + exp10;
    let (mut num, den) = if total_exp >= 0 {
        (
            num_digits * BigInt::from(10).pow(total_exp as u32),
            BigInt::one(),
        )
    } else {
        (num_digits, BigInt::from(10).pow((-total_exp) as u32))
    };
    if neg {
        num = -num;
    }
    frac_normalize(num, den)
}

pub fn frac_instance_num_den(v: &PyObjectRef) -> Option<(BigInt, BigInt)> {
    if let PyObject::Instance { dict, .. } = &*v.borrow() {
        let num = dict.get_str("_numerator")?;
        let den = dict.get_str("_denominator")?;
        let get = |o: &PyObjectRef| -> Option<BigInt> {
            match &*o.borrow() {
                PyObject::Int(n) => Some(n.clone()),
                // `_from_coprime_ints` stores the raw objects (an int
                // subclass like DummyIntegral) — read their int backing.
                PyObject::Instance { .. } => crate::object::int_value_or_backing(o),
                _ => None,
            }
        };
        if let (Some(n), Some(d)) = (get(&num), get(&den)) {
            return Some((n, d));
        }
    }
    None
}

pub fn frac_make(frac_type: &PyObjectRef, num: BigInt, den: BigInt) -> PyResult<PyObjectRef> {
    let (num, den) = frac_normalize(num, den)?;
    let mut dict = AttrMap::new();
    dict.insert_str("_numerator", py_int(num));
    dict.insert_str("_denominator", py_int(den));
    Ok(PyObjectRef::new(PyObject::Instance {
        typ: frac_type.clone(),
        dict,
    }))
}

/// Numeric operand kind for Fraction arithmetic's real-Python coercion
/// rules: `Fraction op int` stays a `Fraction`; `Fraction op float` (or
/// vice versa) coerces the WHOLE operation to plain `float` (matching real
/// `Fraction.__add__`'s own documented behavior); anything else is
/// `NotImplemented` (deferring to the other operand's reflected method).
pub enum FracOperand {
    Frac(BigInt, BigInt),
    Float(f64),
    Other,
}

pub fn frac_operand_of(v: &PyObjectRef) -> FracOperand {
    if let Some((n, d)) = frac_instance_num_den(v) {
        return FracOperand::Frac(n, d);
    }
    let b = v.borrow();
    match &*b {
        PyObject::Int(i) => FracOperand::Frac(i.clone(), BigInt::one()),
        PyObject::Bool(bv) => FracOperand::Frac(BigInt::from(*bv as i64), BigInt::one()),
        PyObject::Float(f) => FracOperand::Float(*f),
        PyObject::Instance { .. } => {
            // An `numbers.Rational`-registered class (e.g. the test suite's
            // `Rat` / `Root`) exposes numerator/denominator attributes —
            // Fraction arithmetic/comparison accepts these.
            drop(b);
            if let (Ok(num), Ok(den)) = (
                v.borrow().get_attribute("numerator"),
                v.borrow().get_attribute("denominator"),
            ) {
                let n = crate::object::int_value_or_backing(&num)
                    .or_else(|| crate::object::to_index(&num).ok());
                let d = crate::object::int_value_or_backing(&den)
                    .or_else(|| crate::object::to_index(&den).ok());
                if let (Some(n), Some(d)) = (n, d) {
                    return FracOperand::Frac(n, d);
                }
            }
            FracOperand::Other
        }
        _ => FracOperand::Other,
    }
}

/// True iff `other` is an exact `int`/`Fraction` (or subclass) — the only
/// Rationals a FORWARD Fraction arithmetic op handles directly (CPython's
/// `_operator_fallbacks` monomorphic arm); everything else defers to the
/// other operand's reflected method.
pub fn frac_forward_ok(other: &PyObjectRef) -> bool {
    if matches!(&*other.borrow(), PyObject::Int(_) | PyObject::Bool(_)) {
        return true;
    }
    if let PyObject::Instance { typ, .. } = &*other.borrow() {
        if typ.is(&get_fraction_type()) {
            return true;
        }
        if let PyObject::Type { mro, .. } = &*typ.borrow() {
            return mro.iter().skip(1).any(|b| b.is(&get_fraction_type()));
        }
    }
    false
}

/// Reflected-op fallback for a `numbers.Real` operand (CPython's reverse
/// `isinstance(a, numbers.Real) -> float(a) op float(self)` arm): invoke the
/// operand's `__float__` and compute `float_op(other_f, self_f)`. Decimal
/// is deliberately excluded (it has `__float__` but refuses mixed arithmetic).
pub fn frac_reflected_float<F: Fn(f64, f64) -> f64>(
    other: &PyObjectRef,
    self_f: f64,
    float_op: F,
) -> Option<PyObjectRef> {
    if instance_to_decval(other).is_some() {
        return None;
    }
    let f = other.borrow().get_attribute("__float__").ok()?;
    let result = crate::object::call_bound_method(f, other.clone(), vec![]).ok()?;
    let af = result.as_f64()?;
    Some(py_float(float_op(af, self_f)))
}

/// Reflected-op fallback for a `numbers.Complex` operand (CPython's reverse
/// `isinstance(a, numbers.Complex) -> complex(a) op complex(self)` arm):
/// invoke the operand's `__complex__` and apply `op` to its parts.
pub fn frac_reflected_complex<F: Fn((f64, f64), (f64, f64)) -> (f64, f64)>(
    other: &PyObjectRef,
    self_f: f64,
    op: F,
) -> Option<PyObjectRef> {
    if instance_to_decval(other).is_some() {
        return None;
    }
    let f = other.borrow().get_attribute("__complex__").ok()?;
    let result = crate::object::call_bound_method(f, other.clone(), vec![]).ok()?;
    let parts = crate::object::as_complex_parts(&*result.borrow())?;
    let (cr, ci) = op(parts, (self_f, 0.0));
    Some(PyObjectRef::imm(PyObject::Complex(cr, ci)))
}

/// True iff `other` is a real `complex` (or a complex-subclass instance) —
/// CPython's forward `isinstance(b, complex)` arm.
pub fn frac_is_complex_operand(other: &PyObjectRef) -> bool {
    if matches!(&*other.borrow(), PyObject::Complex(..)) {
        return true;
    }
    if let PyObject::Instance { typ, .. } = &*other.borrow() {
        return crate::object::native_base_of_type(typ).as_deref() == Some("complex");
    }
    false
}

/// Just the float value of a `numbers.Real`-style operand (its `__float__`).
pub fn frac_reflected_float_value(other: &PyObjectRef) -> Option<f64> {
    let f = other.borrow().get_attribute("__float__").ok()?;
    let result = crate::object::call_bound_method(f, other.clone(), vec![]).ok()?;
    result.as_f64()
}

pub fn frac_self_num_den(self_obj: &PyObjectRef) -> PyResult<(BigInt, BigInt)> {
    frac_instance_num_den(self_obj).ok_or_else(|| PyError::type_error("not a Fraction"))
}

/// Python `float(a) ** float(b)`: a negative base with a non-integral
/// exponent yields a complex result (e.g. `(-1.0) ** 0.5` -> 1j).
pub fn frac_float_pow(base: f64, exp: f64) -> PyObjectRef {
    if base < 0.0 && exp.fract() != 0.0 {
        let mag = (-base).powf(exp);
        let theta = std::f64::consts::PI * exp;
        PyObjectRef::imm(PyObject::Complex(mag * theta.cos(), mag * theta.sin()))
    } else {
        py_float(base.powf(exp))
    }
}

/// Rational `a ** power` for an integer power (CPython's Fraction.__pow__
/// integer branch): a non-negative power raises num/den; a negative power
/// inverts, raising ZeroDivisionError for a zero base.
pub fn frac_rational_pow(an: &BigInt, ad: &BigInt, power: &BigInt) -> PyResult<(BigInt, BigInt)> {
    let p = power.to_u32();
    match p {
        Some(p) => Ok((an.pow(p), ad.pow(p))),
        None if !an.is_zero() => {
            let p = (-power).to_u32().unwrap_or(u32::MAX);
            Ok((ad.pow(p), an.pow(p)))
        }
        None => {
            let p = (-power).to_u32().unwrap_or(u32::MAX);
            Err(PyError::ZeroDivisionError(format!(
                "Fraction({}, 0)",
                ad.pow(p)
            )))
        }
    }
}

pub fn frac_to_f64(num: &BigInt, den: &BigInt) -> f64 {
    if num.is_zero() {
        return 0.0;
    }
    if den.is_zero() {
        return f64::INFINITY;
    }
    let sign = if (num.sign() == num_bigint::Sign::Minus) != (den.sign() == num_bigint::Sign::Minus)
    {
        -1.0
    } else {
        1.0
    };
    let n = num.abs();
    let d = den.abs();
    // Keep ~54 bits of precision and scale both sides DOWN to fit in f64,
    // so huge numerators/denominators don't overflow to inf before dividing
    // (CPython's `int / int` true division semantics for
    // `Fraction.__float__` — `float(F(2*10**400, 3*10**400))` must round
    // to 2/3, not NaN).
    let prec = 54u64;
    let nbits = n.bits();
    let dbits = d.bits();
    let shift_n = nbits.saturating_sub(prec);
    let shift_d = dbits.saturating_sub(prec);
    let n2 = n >> shift_n;
    let d2 = d >> shift_d;
    let ratio = n2.to_f64().unwrap_or(f64::INFINITY) / d2.to_f64().unwrap_or(f64::INFINITY);
    sign * ratio * 2f64.powf(shift_n as f64 - shift_d as f64)
}

/// Exact comparison of `num/den` against an `f64` (CPython's Fraction/float
/// comparisons use the float's exact binary value, so `F(10**23) == 1e23`
/// is False). `None` when NaN is involved.
pub fn frac_cmp_exact(num: &BigInt, den: &BigInt, f: f64) -> Option<std::cmp::Ordering> {
    if f.is_nan() {
        return None;
    }
    if f.is_infinite() {
        // Every finite fraction is less than +inf and greater than -inf.
        return Some(if f.is_sign_positive() {
            std::cmp::Ordering::Less
        } else {
            std::cmp::Ordering::Greater
        });
    }
    if num.is_zero() {
        return Some(0.0f64.partial_cmp(&f).unwrap_or(std::cmp::Ordering::Equal));
    }
    let (fn_, fd) = frac_float_to_ratio(f).ok()?;
    // Compare num/den with fn_/fd_ exactly (cross-multiplied).
    Some((num * &fd).cmp(&(&fn_ * den)))
}

/// Shared binary-op dispatcher: `op` combines two exact `(num, den)` pairs;
/// `float_op` combines two `f64`s for the mixed-with-float coercion case.
pub fn frac_binop(
    args: &[PyObjectRef],
    reflected: bool,
    op: impl Fn(BigInt, BigInt, BigInt, BigInt) -> PyResult<(BigInt, BigInt)>,
    float_op: impl Fn(f64, f64) -> f64,
    complex_op: fn((f64, f64), (f64, f64)) -> (f64, f64),
    py_op: fn(&PyObjectRef, &PyObjectRef) -> PyResult<PyObjectRef>,
) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Err(PyError::type_error("expected 2 arguments"));
    }
    // `self` (args[0]) is always the Fraction whose method this is; for a
    // reflected call (`__radd__` etc.) `self` is semantically the RIGHT
    // operand of `other OP self`, so `op`'s arguments are swapped below
    // rather than swapping `an`/`ad` here.
    let (an, ad) = frac_self_num_den(&args[0])?;
    match frac_operand_of(&args[1]) {
        FracOperand::Frac(bn, bd) => {
            if !reflected && !frac_forward_ok(&args[1]) {
                return Ok(py_not_implemented());
            }
            let (rn, rd) = if reflected {
                op(bn, bd, an, ad)?
            } else {
                op(an, ad, bn, bd)?
            };
            frac_make(&get_fraction_type(), rn, rd)
        }
        FracOperand::Float(bf) => {
            let af = frac_to_f64(&an, &ad);
            Ok(py_float(if reflected {
                float_op(bf, af)
            } else {
                float_op(af, bf)
            }))
        }
        FracOperand::Other => {
            if reflected {
                if let Some(r) = frac_reflected_float(&args[1], frac_to_f64(&an, &ad), float_op) {
                    return Ok(r);
                }
                if let Some(r) = frac_reflected_complex(&args[1], frac_to_f64(&an, &ad), complex_op)
                {
                    return Ok(r);
                }
            } else if frac_is_complex_operand(&args[1]) {
                // `isinstance(b, complex)`: CPython's forward fallback
                // `fallback_operator(float(a), b)`.
                let af = frac_to_f64(&an, &ad);
                return py_op(&py_float(af), &args[1]);
            }
            Ok(py_not_implemented())
        }
    }
}
/// Python `divmod` (floor division): returns (q, r) with 0 <= r < |d| and
/// r matching d's sign for positive d (CPython's `divmod` semantics, which
/// the round-ties-to-even digit generation depends on for negative inputs).
pub fn floor_div_rem(n: BigInt, d: &BigInt) -> (BigInt, BigInt) {
    let q = &n / d;
    let r = &n % d;
    if r != BigInt::zero() && (r.sign() != d.sign()) {
        (q - 1, r + d)
    } else {
        (q, r)
    }
}

/// Round rational n/d to the nearest multiple of 10**exponent, ties-to-even
/// (port of CPython's fractions._round_to_exponent).
pub fn frac_round_to_exponent(n: BigInt, d: BigInt, exponent: i64) -> (bool, BigInt) {
    let (n, d) = if exponent >= 0 {
        (n, d * BigInt::from(10).pow(exponent as u32))
    } else {
        (n * BigInt::from(10).pow((-exponent) as u32), d)
    };
    let half = &d >> 1;
    let (mut q, r) = floor_div_rem(&n + &half, &d);
    if r == BigInt::zero() && (&d & BigInt::from(1)) == BigInt::zero() {
        q &= BigInt::from(-2);
    }
    let sign = n.sign() == num_bigint::Sign::Minus;
    (sign, q.abs())
}

/// Round rational n/d to `figures` significant figures (port of CPython's
/// fractions._round_to_figures).
pub fn frac_round_to_figures(n: BigInt, d: BigInt, figures: usize) -> (bool, BigInt, i64) {
    if n == BigInt::zero() {
        return (false, BigInt::zero(), 1 - figures as i64);
    }
    let str_n = n.abs().to_string();
    let str_d = d.to_string();
    let m = str_n.len() as i64 - str_d.len() as i64
        + if str_d.as_str() <= str_n.as_str() {
            1
        } else {
            0
        };
    let exponent = m - figures as i64;
    let (sign, mut significand) = frac_round_to_exponent(n, d, exponent);
    let mut exponent = exponent;
    if significand.to_string().len() as i64 == figures as i64 + 1 {
        significand /= 10;
        exponent += 1;
    }
    (sign, significand, exponent)
}

/// A parsed general (no-presentation-type) format spec — port of CPython's
/// `_GENERAL_FORMAT_SPECIFICATION_MATCHER`.
pub struct FracGeneralSpec {
    fill: char,
    align: char,
    sign: char,
    alt: bool,
    width: usize,
    thousands: Option<char>,
}

/// Parse a general format spec; `None` if the spec does not fullmatch
/// (in which case it should be tried as a float-style spec).
pub fn frac_parse_general_spec(spec: &str) -> Option<FracGeneralSpec> {
    let chars: Vec<char> = spec.chars().collect();
    let len = chars.len();
    let mut idx = 0;
    let mut fill = ' ';
    let mut align = '>';
    if idx + 1 < len && matches!(chars[idx + 1], '<' | '>' | '^' | '=') {
        fill = chars[idx];
        align = chars[idx + 1];
        idx += 2;
    } else if idx < len && matches!(chars[idx], '<' | '>' | '^' | '=') {
        align = chars[idx];
        idx += 1;
    }
    let mut sign = '-';
    if idx < len && matches!(chars[idx], '+' | '-' | ' ') {
        sign = chars[idx];
        idx += 1;
    }
    let mut alt = false;
    if idx < len && chars[idx] == '#' {
        alt = true;
        idx += 1;
    }
    let mut width = 0usize;
    if idx < len && chars[idx] == '0' {
        // '0' alone is a width; '0' followed by digits cannot fullmatch.
        if idx + 1 < len && chars[idx + 1].is_ascii_digit() {
            return None;
        }
        idx += 1;
    } else if idx < len && chars[idx].is_ascii_digit() {
        while idx < len && chars[idx].is_ascii_digit() {
            width = width * 10 + chars[idx].to_digit(10).unwrap() as usize;
            idx += 1;
        }
    }
    let mut thousands = None;
    if idx < len && (chars[idx] == ',' || chars[idx] == '_') {
        thousands = Some(chars[idx]);
        idx += 1;
    }
    if idx < len {
        return None;
    }
    Some(FracGeneralSpec {
        fill,
        align,
        sign,
        alt,
        width,
        thousands,
    })
}

pub fn frac_group_digits(s: &str, sep: Option<char>) -> String {
    match sep {
        None => s.to_string(),
        Some(sep) => {
            let mut g = String::new();
            let bytes: Vec<char> = s.chars().collect();
            for (i, c) in bytes.iter().enumerate() {
                if i > 0 && (bytes.len() - i) % 3 == 0 {
                    g.push(sep);
                }
                g.push(*c);
            }
            g
        }
    }
}

/// Format a Fraction with a general (no presentation type) spec — port of
/// CPython's `Fraction._format_general`.
pub fn frac_format_general(num: BigInt, den: &BigInt, p: &FracGeneralSpec) -> PyResult<String> {
    let pos_sign = if p.sign == '-' {
        String::new()
    } else {
        p.sign.to_string()
    };
    let sign_out = if num < BigInt::zero() {
        "-".to_string()
    } else {
        pos_sign
    };
    let an = num.abs();
    let body = if *den > BigInt::one() || p.alt {
        format!(
            "{}/{}",
            frac_group_digits(&an.to_string(), p.thousands),
            frac_group_digits(&den.to_string(), p.thousands)
        )
    } else {
        frac_group_digits(&an.to_string(), p.thousands)
    };
    let padding_len = p.width.saturating_sub(sign_out.len() + body.len());
    let padding = p.fill.to_string().repeat(padding_len);
    Ok(match p.align {
        '<' => format!("{}{}{}", sign_out, body, padding),
        '^' => {
            let half = padding_len / 2;
            format!(
                "{}{}{}{}",
                &padding[..half],
                sign_out,
                body,
                &padding[half..]
            )
        }
        '=' => format!("{}{}{}", sign_out, padding, body),
        _ => format!("{}{}{}", padding, sign_out, body),
    })
}

