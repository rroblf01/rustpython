use crate::object::*;
use std::collections::HashMap;
use num_bigint::{BigInt, Sign};
use num_traits::{Zero, One, Signed, ToPrimitive};
use crate::modules::data::fractions::{frac_instance_num_den, frac_to_f64, frac_normalize};
use crate::modules::data::decimal_types::build_decimal_type;
// ===================== Real decimal.Decimal =====================
//
// Arbitrary-precision decimal arithmetic per (a practical subset of) IBM's
// General Decimal Arithmetic Specification, the same spec CPython's own
// `decimal` module follows. A Decimal value is sign/coefficient/exponent
// (or one of the special states NaN/sNaN/Infinity); the coefficient is a
// `BigInt` so precision is genuinely unbounded, matching real semantics
// (unlike the previous stub, which just wrapped the constructor argument in
// a string with no arithmetic at all).
//
// Scope: construction (str/int/float/Decimal/tuple), correct string
// formatting, +-*/ (with context precision/rounding), //, %, **  for integer
// exponents, unary -/+/abs, comparisons, a usable (if approximate) hash,
// quantize/normalize/as_tuple/is_*, and a Context type with
// getcontext/setcontext/localcontext. Not implemented: exp/ln/log10/sqrt,
// non-integer power, signal traps/flags (rounding happens silently, as if
// no traps were enabled — only InvalidOperation/DivisionByZero on truly
// undefined operations actually raise).

#[derive(Clone, PartialEq, Debug)]
pub enum DecSpecial {
    Finite,
    QNaN,
    SNaN,
    Infinity,
}

#[derive(Clone, Debug)]
pub struct DecValue {
    pub special: DecSpecial,
    pub sign: bool,                // true = negative
    pub coeff: num_bigint::BigInt, // non-negative significand; 0 for NaN/Infinity
    pub exp: i64,                  // meaningless for NaN/Infinity
}

impl DecValue {
    pub fn zero() -> Self {
        DecValue {
            special: DecSpecial::Finite,
            sign: false,
            coeff: num_bigint::BigInt::from(0),
            exp: 0,
        }
    }
    pub fn nan() -> Self {
        DecValue {
            special: DecSpecial::QNaN,
            sign: false,
            coeff: num_bigint::BigInt::from(0),
            exp: 0,
        }
    }
    pub fn infinity(sign: bool) -> Self {
        DecValue {
            special: DecSpecial::Infinity,
            sign,
            coeff: num_bigint::BigInt::from(0),
            exp: 0,
        }
    }
    pub fn is_zero(&self) -> bool {
        self.special == DecSpecial::Finite && num_traits::Zero::is_zero(&self.coeff)
    }
    pub fn is_nan(&self) -> bool {
        matches!(self.special, DecSpecial::QNaN | DecSpecial::SNaN)
    }
}

pub fn parse_decimal_str(raw: &str) -> Option<DecValue> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    let mut sign = false;
    let rest = if let Some(r) = s.strip_prefix('+') {
        r
    } else if let Some(r) = s.strip_prefix('-') {
        sign = true;
        r
    } else {
        s
    };
    if rest.is_empty() {
        return None;
    }
    let rest_lower = rest.to_ascii_lowercase();
    if rest_lower == "inf" || rest_lower == "infinity" {
        return Some(DecValue::infinity(sign));
    }
    if let Some(digits_part) = rest_lower.strip_prefix("snan") {
        let coeff = if digits_part.is_empty() {
            num_bigint::BigInt::from(0)
        } else {
            num_bigint::BigInt::parse_bytes(digits_part.as_bytes(), 10)?
        };
        return Some(DecValue {
            special: DecSpecial::SNaN,
            sign,
            coeff,
            exp: 0,
        });
    }
    if let Some(digits_part) = rest_lower.strip_prefix("nan") {
        let coeff = if digits_part.is_empty() {
            num_bigint::BigInt::from(0)
        } else {
            num_bigint::BigInt::parse_bytes(digits_part.as_bytes(), 10)?
        };
        return Some(DecValue {
            special: DecSpecial::QNaN,
            sign,
            coeff,
            exp: 0,
        });
    }
    let (mantissa_part, exp_part) = match rest.find(['e', 'E']) {
        Some(idx) => (&rest[..idx], Some(&rest[idx + 1..])),
        None => (rest, None),
    };
    if mantissa_part.is_empty() {
        return None;
    }
    let (int_part, frac_part) = match mantissa_part.find('.') {
        Some(idx) => (&mantissa_part[..idx], &mantissa_part[idx + 1..]),
        None => (mantissa_part, ""),
    };
    if int_part.is_empty() && frac_part.is_empty() {
        return None;
    }
    if !int_part.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    if !frac_part.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let digits_str = format!("{}{}", int_part, frac_part);
    let digits_str = if digits_str.is_empty() {
        "0".to_string()
    } else {
        digits_str
    };
    let coeff = num_bigint::BigInt::parse_bytes(digits_str.as_bytes(), 10)?;
    let mut exp: i64 = -(frac_part.len() as i64);
    if let Some(exp_str) = exp_part {
        let exp_str = exp_str.trim();
        if exp_str.is_empty() {
            return None;
        }
        let extra: i64 = exp_str.parse().ok()?;
        exp += extra;
    }
    Some(DecValue {
        special: DecSpecial::Finite,
        sign,
        coeff,
        exp,
    })
}

/// Largest `k` such that `b^k` divides `n` (for prime `b`).
pub fn factor_power_of(n: &num_bigint::BigUint, b: u8) -> u32 {
    let mut v = n.clone();
    let mut k = 0u32;
    while &v % num_bigint::BigUint::from(b) == num_bigint::BigUint::from(0u8) {
        v /= num_bigint::BigUint::from(b);
        k += 1;
    }
    k
}

pub fn decval_from_f64(f: f64) -> DecValue {
    float_to_decvalue(f)
}

/// The EXACT decimal value of an f64 (CPython's `Decimal(float)` and
/// `Decimal.from_float(f)` both produce the exact binary value, not the
/// shortest repr): `m * 2**e` written as `m * 5**k / 10**k`, normalized by
/// removing trailing factors of 10.
pub fn float_to_decvalue(f: f64) -> DecValue {
    if f.is_nan() {
        return DecValue::nan();
    }
    if f.is_infinite() {
        return DecValue::infinity(f < 0.0);
    }
    if f == 0.0 {
        return DecValue {
            special: DecSpecial::Finite,
            sign: f.is_sign_negative(),
            coeff: num_bigint::BigInt::from(0),
            exp: 0,
        };
    }
    let bits = f.to_bits();
    let sign = (bits >> 63) != 0;
    let biased = ((bits >> 52) & 0x7ff) as i64;
    let mantissa = bits & 0x000f_ffff_ffff_ffff;
    let (m, e) = if biased == 0 {
        (mantissa, -1074i64)
    } else {
        ((1u64 << 52) | mantissa, biased - 1023 - 52)
    };
    let coeff0 = num_bigint::BigInt::from(m);
    let (mut coeff, mut exp) = if e >= 0 {
        (coeff0 << (e as u32), 0i64)
    } else {
        let k = (-e) as u32;
        (coeff0 * num_bigint::BigInt::from(5u32).pow(k), -(k as i64))
    };
    let ten = num_bigint::BigInt::from(10);
    while coeff != num_bigint::BigInt::zero() && (&coeff % &ten).is_zero() {
        coeff /= &ten;
        exp += 1;
    }
    DecValue {
        special: DecSpecial::Finite,
        sign,
        coeff,
        exp,
    }
}

pub fn ten_pow(n: i64) -> num_bigint::BigInt {
    if n <= 0 {
        return num_bigint::BigInt::from(1);
    }
    num_bigint::BigInt::from(10).pow(n as u32)
}

pub fn digit_count(coeff: &num_bigint::BigInt) -> usize {
    if num_traits::Zero::is_zero(coeff) {
        return 1;
    }
    coeff.to_string().len()
}

/// CPython's decimal-to-string algorithm (IBM spec `to-scientific-string`):
/// plain notation when the exponent is small enough, scientific otherwise.
pub fn format_decvalue(v: &DecValue) -> String {
    let sign_str = if v.sign { "-" } else { "" };
    match v.special {
        DecSpecial::Infinity => return format!("{}Infinity", sign_str),
        DecSpecial::QNaN => {
            let digits = if num_traits::Zero::is_zero(&v.coeff) {
                String::new()
            } else {
                v.coeff.to_string()
            };
            return format!("{}NaN{}", sign_str, digits);
        }
        DecSpecial::SNaN => {
            let digits = if num_traits::Zero::is_zero(&v.coeff) {
                String::new()
            } else {
                v.coeff.to_string()
            };
            return format!("{}sNaN{}", sign_str, digits);
        }
        DecSpecial::Finite => {}
    }
    let digits = if num_traits::Zero::is_zero(&v.coeff) {
        "0".to_string()
    } else {
        v.coeff.to_string()
    };
    let leading = digits.len() as i64;
    let adjusted_exp = v.exp + leading - 1;
    if v.exp <= 0 && adjusted_exp >= -6 {
        let body = if v.exp == 0 {
            digits
        } else if leading <= -v.exp {
            format!("0.{}{}", "0".repeat((-v.exp - leading) as usize), digits)
        } else {
            let split = (leading + v.exp) as usize;
            format!("{}.{}", &digits[..split], &digits[split..])
        };
        format!("{}{}", sign_str, body)
    } else {
        let body = if leading == 1 {
            digits.clone()
        } else {
            format!("{}.{}", &digits[..1], &digits[1..])
        };
        let exp_sign = if adjusted_exp >= 0 { "+" } else { "-" };
        format!("{}{}E{}{}", sign_str, body, exp_sign, adjusted_exp.abs())
    }
}

thread_local! {
    pub static DECIMAL_TYPE: std::cell::RefCell<Option<PyObjectRef>> = std::cell::RefCell::new(None);
    pub static DECIMAL_CONTEXT_TYPE: std::cell::RefCell<Option<PyObjectRef>> = std::cell::RefCell::new(None);
    pub static DECIMAL_CURRENT_CONTEXT: std::cell::RefCell<(usize, String)> = std::cell::RefCell::new((28, "ROUND_HALF_EVEN".to_string()));
    pub static DECIMAL_IS_BASIC: std::cell::RefCell<bool> = std::cell::RefCell::new(false);
    pub static FRACTION_TYPE: std::cell::RefCell<Option<PyObjectRef>> = std::cell::RefCell::new(None);
}

/// The plain `Fraction` type (not a subclass) — Fraction arithmetic always
/// returns plain `Fraction` instances, matching CPython.
pub fn get_fraction_type() -> PyObjectRef {
    FRACTION_TYPE.with(|c| c.borrow().clone().unwrap())
}

pub fn current_decimal_context() -> (usize, String) {
    DECIMAL_CURRENT_CONTEXT.with(|c| c.borrow().clone())
}
pub fn current_is_basic() -> bool {
    DECIMAL_IS_BASIC.with(|c| *c.borrow())
}

pub const DEC_SIGN_KEY: &str = "_sign";
pub const DEC_COEFF_KEY: &str = "_coeff";
pub const DEC_EXP_KEY: &str = "_exp";
pub const DEC_SPECIAL_KEY: &str = "_special";

pub fn special_to_str(s: &DecSpecial) -> &'static str {
    match s {
        DecSpecial::Finite => "",
        DecSpecial::QNaN => "n",
        DecSpecial::SNaN => "N",
        DecSpecial::Infinity => "F",
    }
}
pub fn special_from_str(s: &str) -> DecSpecial {
    match s {
        "n" => DecSpecial::QNaN,
        "N" => DecSpecial::SNaN,
        "F" => DecSpecial::Infinity,
        _ => DecSpecial::Finite,
    }
}

pub fn decval_to_instance(v: &DecValue) -> PyObjectRef {
    let typ = get_decimal_type();
    let mut dict = AttrMap::new();
    dict.insert(DEC_SIGN_KEY.to_string(), py_bool(v.sign));
    dict.insert(DEC_COEFF_KEY.to_string(), py_int(v.coeff.clone()));
    dict.insert(DEC_EXP_KEY.to_string(), py_int(v.exp));
    dict.insert(
        DEC_SPECIAL_KEY.to_string(),
        py_str(special_to_str(&v.special)),
    );
    PyObjectRef::new(PyObject::Instance { typ, dict })
}

pub fn instance_to_decval(obj: &PyObjectRef) -> Option<DecValue> {
    if let PyObject::Instance { dict, .. } = &*obj.borrow() {
        let sign = dict.get(DEC_SIGN_KEY)?.truthy();
        let coeff = match &*dict.get(DEC_COEFF_KEY)?.borrow() {
            PyObject::Int(i) => i.clone(),
            _ => return None,
        };
        let exp = dict.get(DEC_EXP_KEY)?.as_i64().unwrap_or(0);
        let special = special_from_str(&dict.get(DEC_SPECIAL_KEY)?.str());
        Some(DecValue {
            special,
            sign,
            coeff,
            exp,
        })
    } else {
        None
    }
}

/// Coerce a constructor argument (str/int/float/Decimal/tuple) into a DecValue.
pub fn decval_from_pyobject(v: &PyObjectRef) -> PyResult<DecValue> {
    if let Some(existing) = instance_to_decval(v) {
        return Ok(existing);
    }
    match &*v.borrow() {
        PyObject::Str(s) => parse_decimal_str(s).ok_or_else(|| {
            PyError::Exception(
                "InvalidOperation".to_string(),
                PyObjectRef::new(PyObject::Exception {
                    typ: "InvalidOperation".to_string(),
                    args: vec![py_str(&format!("invalid literal for Decimal: '{}'", s))],
                    cause: None,
                    suppress_context: false,
                    context: None,
                    traceback: None,
                    extra: None,
                }),
            )
        }),
        PyObject::Int(i) => {
            let sign = num_traits::Signed::is_negative(i);
            Ok(DecValue {
                special: DecSpecial::Finite,
                sign,
                coeff: num_traits::Signed::abs(i),
                exp: 0,
            })
        }
        PyObject::Bool(b) => Ok(DecValue {
            special: DecSpecial::Finite,
            sign: false,
            coeff: num_bigint::BigInt::from(if *b { 1 } else { 0 }),
            exp: 0,
        }),
        PyObject::Float(f) => Ok(decval_from_f64(*f)),
        PyObject::Tuple(parts) => {
            if parts.len() != 3 {
                return Err(PyError::value_error(
                    "argument must be a sequence of length 3",
                ));
            }
            let sign = parts[0].as_i64().unwrap_or(0) != 0;
            let digit_items: Vec<PyObjectRef> = match &*parts[1].borrow() {
                PyObject::Tuple(d) | PyObject::List(d) => d.clone(),
                _ => return Err(PyError::value_error("digits must be a sequence of ints")),
            };
            let mut digits_str = String::new();
            for d in &digit_items {
                digits_str.push_str(&d.as_i64().unwrap_or(0).to_string());
            }
            if digits_str.is_empty() {
                digits_str.push('0');
            }
            match &*parts[2].borrow() {
                PyObject::Str(s) if s == "F" => Ok(DecValue::infinity(sign)),
                PyObject::Str(s) if s == "n" || s == "N" => {
                    let coeff = num_bigint::BigInt::parse_bytes(digits_str.as_bytes(), 10)
                        .unwrap_or_default();
                    Ok(DecValue {
                        special: special_from_str(s),
                        sign,
                        coeff,
                        exp: 0,
                    })
                }
                _ => {
                    let exp = parts[2].as_i64().unwrap_or(0);
                    let coeff = num_bigint::BigInt::parse_bytes(digits_str.as_bytes(), 10)
                        .unwrap_or_default();
                    Ok(DecValue {
                        special: DecSpecial::Finite,
                        sign,
                        coeff,
                        exp,
                    })
                }
            }
        }
        PyObject::None => Ok(DecValue::zero()),
        // A `fractions.Fraction` operand (e.g. `Decimal('1') < Fraction(1,2)` /
        // `Decimal('1001.0') == Fraction(2002, 2)` — CPython's numeric tower
        // converts the Fraction to a Decimal for the comparison) — was
        // hitting the `_ =>` "unsupported type" TypeError below.
        _ => {
            if let Some((num, den)) = frac_instance_num_den(v) {
                let (sign, coeff) = if num.sign() == num_bigint::Sign::Minus {
                    (true, (-num.clone()).to_biguint().unwrap_or_default())
                } else {
                    (false, num.to_biguint().unwrap_or_default())
                };
                let den_b = den.to_biguint().unwrap_or_default();
                // value = coeff/den_b; express exactly as X * 10**e by
                // factoring den_b = 2**twos * 5**fives and clearing the
                // extra 2s/5s against a power of 10:
                //   fives >= twos: X = coeff * 2**(fives-twos), e = -fives
                //   twos  >  fives: X = coeff * 5**(twos-fives), e = -twos
                let (twos, fives) = (factor_power_of(&den_b, 2), factor_power_of(&den_b, 5));
                let mut den_rem = den_b;
                for _ in 0..twos {
                    den_rem /= 2u8;
                }
                for _ in 0..fives {
                    den_rem /= 5u8;
                }
                // den_rem must be 1 now (any 2s/5s removed); remaining
                // factors make it non-terminating — CPython raises TypeError
                // for Decimal(Fraction) with non-terminating denominator, so
                // that statistics._convert can fall back to
                // Decimal(numerator)/Decimal(denominator) which does the
                // correctly-rounded division.
                if den_rem == num_bigint::BigUint::from(1u8) {
                    let (scaled, exp) = if fives >= twos {
                        (
                            coeff * num_bigint::BigUint::from(2u8).pow((fives - twos) as u32),
                            -(fives as i64),
                        )
                    } else {
                        (
                            coeff * num_bigint::BigUint::from(5u8).pow((twos - fives) as u32),
                            -(twos as i64),
                        )
                    };
                    Ok(DecValue {
                        special: DecSpecial::Finite,
                        sign,
                        coeff: scaled.into(),
                        exp,
                    })
                } else {
                    return Err(PyError::type_error(
                        "conversion from Fraction to Decimal is not supported",
                    ));
                }
            } else {
                Err(PyError::type_error(
                    "conversion from unsupported type to Decimal",
                ))
            }
        }
    }
}

pub fn round_decvalue(v: &DecValue, precision: usize, rounding: &str) -> DecValue {
    if v.special != DecSpecial::Finite {
        return v.clone();
    }
    let ndigits = digit_count(&v.coeff);
    if ndigits <= precision {
        return v.clone();
    }
    let drop = ndigits - precision;
    let divisor = ten_pow(drop as i64);
    let q = &v.coeff / &divisor;
    let r = &v.coeff % &divisor;
    let new_exp = v.exp + drop as i64;
    let twice_r = &r * num_bigint::BigInt::from(2);
    let round_up = match rounding {
        "ROUND_HALF_UP" => twice_r >= divisor,
        "ROUND_HALF_DOWN" => twice_r > divisor,
        "ROUND_HALF_EVEN" => {
            use std::cmp::Ordering;
            match twice_r.cmp(&divisor) {
                Ordering::Greater => true,
                Ordering::Less => false,
                Ordering::Equal => (&q % 2) != num_bigint::BigInt::from(0),
            }
        }
        "ROUND_UP" => !num_traits::Zero::is_zero(&r),
        "ROUND_DOWN" => false,
        "ROUND_CEILING" => !num_traits::Zero::is_zero(&r) && !v.sign,
        "ROUND_FLOOR" => !num_traits::Zero::is_zero(&r) && v.sign,
        "ROUND_05UP" => {
            !num_traits::Zero::is_zero(&r) && {
                let last = &q % 10;
                last == num_bigint::BigInt::from(0) || last == num_bigint::BigInt::from(5)
            }
        }
        _ => {
            use std::cmp::Ordering;
            match twice_r.cmp(&divisor) {
                Ordering::Greater => true,
                Ordering::Less => false,
                Ordering::Equal => (&q % 2) != num_bigint::BigInt::from(0),
            }
        }
    };
    let final_q = if round_up { q + 1 } else { q };
    DecValue {
        special: DecSpecial::Finite,
        sign: v.sign,
        coeff: final_q,
        exp: new_exp,
    }
}

pub fn round_to_context(v: DecValue) -> DecValue {
    let (precision, rounding) = current_decimal_context();
    round_decvalue(&v, precision, &rounding)
}

pub fn decval_align(a: &DecValue, b: &DecValue) -> (num_bigint::BigInt, num_bigint::BigInt, i64) {
    let exp = a.exp.min(b.exp);
    let a_scaled = &a.coeff * ten_pow(a.exp - exp);
    let b_scaled = &b.coeff * ten_pow(b.exp - exp);
    (a_scaled, b_scaled, exp)
}

pub fn decimal_invalid_op(msg: &str) -> PyError {
    PyError::Exception(
        "InvalidOperation".to_string(),
        PyObjectRef::new(PyObject::Exception {
            typ: "InvalidOperation".to_string(),
            args: vec![py_str(msg)],
            cause: None,
            suppress_context: false,
            context: None,
            traceback: None,
            extra: None,
        }),
    )
}
pub fn decimal_division_by_zero(msg: &str) -> PyError {
    PyError::Exception(
        "DivisionByZero".to_string(),
        PyObjectRef::new(PyObject::Exception {
            typ: "DivisionByZero".to_string(),
            args: vec![py_str(msg)],
            cause: None,
            suppress_context: false,
            context: None,
            traceback: None,
            extra: None,
        }),
    )
}

pub fn decimal_add(a: &DecValue, b: &DecValue) -> PyResult<DecValue> {
    // sNaN must signal InvalidOperation
    if a.special == DecSpecial::SNaN || b.special == DecSpecial::SNaN {
        return Err(decimal_invalid_op("sNaN in operation"));
    }
    if a.special == DecSpecial::QNaN || b.special == DecSpecial::QNaN {
        let src = if a.special == DecSpecial::QNaN { a } else { b };
        return Ok(DecValue {
            special: DecSpecial::QNaN,
            sign: src.sign,
            coeff: src.coeff.clone(),
            exp: 0,
        });
    }
    if a.special == DecSpecial::Infinity || b.special == DecSpecial::Infinity {
        if a.special == DecSpecial::Infinity
            && b.special == DecSpecial::Infinity
            && a.sign != b.sign
        {
            if current_is_basic() {
                return Err(decimal_invalid_op("(+Infinity) + (-Infinity)"));
            } else {
                return Ok(DecValue::nan());
            }
        }
        return Ok(DecValue::infinity(if a.special == DecSpecial::Infinity {
            a.sign
        } else {
            b.sign
        }));
    }
    let (as_, bs, exp) = decval_align(a, b);
    let sum = (if a.sign { -as_ } else { as_ }) + (if b.sign { -bs } else { bs });
    let sign = num_traits::Signed::is_negative(&sum);
    let result = DecValue {
        special: DecSpecial::Finite,
        sign,
        coeff: num_traits::Signed::abs(&sum),
        exp,
    };
    Ok(round_to_context(result))
}

pub fn decimal_negate(v: &DecValue) -> DecValue {
    let mut r = v.clone();
    if r.special == DecSpecial::Finite || r.special == DecSpecial::Infinity {
        r.sign = !r.sign;
    }
    r
}

pub fn decimal_sub(a: &DecValue, b: &DecValue) -> PyResult<DecValue> {
    decimal_add(a, &decimal_negate(b))
}

pub fn decimal_mul(a: &DecValue, b: &DecValue) -> PyResult<DecValue> {
    if a.is_nan() || b.is_nan() {
        let src = if a.is_nan() { a } else { b };
        return Ok(DecValue {
            special: DecSpecial::QNaN,
            sign: src.sign,
            coeff: src.coeff.clone(),
            exp: 0,
        });
    }
    let sign = a.sign != b.sign;
    if a.special == DecSpecial::Infinity || b.special == DecSpecial::Infinity {
        if a.is_zero() || b.is_zero() {
            return Err(decimal_invalid_op("(+/-Infinity) * 0"));
        }
        return Ok(DecValue::infinity(sign));
    }
    let result = DecValue {
        special: DecSpecial::Finite,
        sign,
        coeff: &a.coeff * &b.coeff,
        exp: a.exp + b.exp,
    };
    Ok(round_to_context(result))
}

pub fn decimal_div(a: &DecValue, b: &DecValue) -> PyResult<DecValue> {
    if a.is_nan() || b.is_nan() {
        let src = if a.is_nan() { a } else { b };
        return Ok(DecValue {
            special: DecSpecial::QNaN,
            sign: src.sign,
            coeff: src.coeff.clone(),
            exp: 0,
        });
    }
    let sign = a.sign != b.sign;
    if a.special == DecSpecial::Infinity && b.special == DecSpecial::Infinity {
        return Err(decimal_invalid_op("(+/-Infinity) / (+/-Infinity)"));
    }
    if a.special == DecSpecial::Infinity {
        return Ok(DecValue::infinity(sign));
    }
    if b.special == DecSpecial::Infinity {
        return Ok(DecValue {
            special: DecSpecial::Finite,
            sign,
            coeff: num_bigint::BigInt::from(0),
            exp: 0,
        });
    }
    if b.is_zero() {
        if a.is_zero() {
            return Err(decimal_invalid_op("0 / 0"));
        }
        return Err(decimal_division_by_zero("division by zero"));
    }
    if a.is_zero() {
        return Ok(round_to_context(DecValue {
            special: DecSpecial::Finite,
            sign,
            coeff: num_bigint::BigInt::from(0),
            exp: a.exp - b.exp,
        }));
    }
    let (precision, rounding) = current_decimal_context();
    // Scale with enough guard digits (precision+10) so a single final
    // rounding to `precision` is correctly rounded. The previous code used
    // guard = precision+digit_count+2 and performed TWO roundings (first to
    // guard via `if raw_r*2>=b.coeff`, then to precision) – double rounding
    // gave off-by-one ulp errors (e.g. 1/15 rounded to 28 digits was
    // 0.0666...666 instead of ...667, making harmonic_mean([15,30,60,60])
    // 29.999... not 30). Keeping the raw truncated quotient and rounding
    // once directly to precision avoids that.
    let guard = precision as i64 + 10;
    let scaled_num = &a.coeff * ten_pow(guard);
    let raw_q = &scaled_num / &b.coeff;
    let raw_exp = a.exp - b.exp - guard;
    let result = DecValue {
        special: DecSpecial::Finite,
        sign,
        coeff: raw_q,
        exp: raw_exp,
    };
    Ok(round_decvalue(&result, precision, &rounding))
}

pub fn decimal_compare(a: &DecValue, b: &DecValue) -> Option<std::cmp::Ordering> {
    use std::cmp::Ordering;
    if a.is_nan() || b.is_nan() {
        return None;
    }
    match (&a.special, &b.special) {
        (DecSpecial::Infinity, DecSpecial::Infinity) => {
            return Some(if a.sign == b.sign {
                Ordering::Equal
            } else if a.sign {
                Ordering::Less
            } else {
                Ordering::Greater
            });
        }
        (DecSpecial::Infinity, _) => {
            return Some(if a.sign {
                Ordering::Less
            } else {
                Ordering::Greater
            })
        }
        (_, DecSpecial::Infinity) => {
            return Some(if b.sign {
                Ordering::Greater
            } else {
                Ordering::Less
            })
        }
        _ => {}
    }
    if a.is_zero() && b.is_zero() {
        return Some(Ordering::Equal);
    }
    // Different signs decide immediately.
    if a.sign != b.sign {
        return Some(if a.sign {
            Ordering::Less
        } else {
            Ordering::Greater
        });
    }
    // Same sign: compare MAGNITUDES. The leading-digit exponent
    // `digit_count(coeff) + exp` decides when the values don't overlap;
    // only values with the SAME order of magnitude need exact alignment.
    // (The previous `decval_align` unconditional scaling blew up on huge
    // exponents — e.g. D('-1e425000000') < 0 computed 10**425000000.)
    let a_zero = a.is_zero();
    let b_zero = b.is_zero();
    let mag = |v: &DecValue| digit_count(&v.coeff) as i64 + v.exp;
    let ord = if a_zero {
        Ordering::Less
    }
    // |a| = 0 < |b| (b nonzero)
    else if b_zero {
        Ordering::Greater
    } else {
        let (ma, mb) = (mag(a), mag(b));
        if ma != mb {
            if ma < mb {
                Ordering::Less
            } else {
                Ordering::Greater
            }
        } else {
            let (as_, bs, _) = decval_align(a, b);
            as_.cmp(&bs)
        }
    };
    Some(if a.sign { ord.reverse() } else { ord })
}

pub fn decval_to_f64(v: &DecValue) -> f64 {
    match v.special {
        DecSpecial::Infinity => {
            if v.sign {
                f64::NEG_INFINITY
            } else {
                f64::INFINITY
            }
        }
        DecSpecial::QNaN | DecSpecial::SNaN => f64::NAN,
        DecSpecial::Finite => {
            // Parse the exact decimal string rather than coeff as f64 times
            // 10^exp — that separate multiplication introduces float error
            // (e.g. 12345.0 * 0.01 != 123.45 exactly), whereas Rust's own
            // string-to-f64 parsing correctly rounds to the nearest float.
            format!("{}{}e{}", if v.sign { "-" } else { "" }, v.coeff, v.exp)
                .parse()
                .unwrap_or(0.0)
        }
    }
}

/// Extract an object's numeric VALUE as `(real, imag)` parts, covering the
/// native numeric variants PLUS `fractions.Fraction` and `decimal.Decimal`
/// instances — real CPython's numeric tower compares all of these by value
/// (`Fraction(2002,2) == 1001+0j` and `Decimal('1001.0') == 1001+0j` are
/// both True). Used by the cross-type equality path in `PyObject::equals`.
pub fn numeric_parts_from_ref(obj: &PyObjectRef) -> Option<(f64, f64)> {
    let borrowed = obj.borrow();
    match &*borrowed {
        PyObject::Complex(re, im) => Some((*re, *im)),
        PyObject::Int(n) => n.to_f64().map(|f| (f, 0.0)),
        PyObject::Float(f) => Some((*f, 0.0)),
        PyObject::Bool(b) => Some((if *b { 1.0 } else { 0.0 }, 0.0)),
        PyObject::Instance { .. } => {
            if let Some((num, den)) = frac_instance_num_den(obj) {
                Some((frac_to_f64(&num, &den), 0.0))
            } else if let Some(v) = instance_to_decval(obj) {
                Some((decval_to_f64(&v), 0.0))
            } else {
                None
            }
        }
        _ => None,
    }
}

pub fn normalize_decval(v: &DecValue) -> DecValue {
    if v.special != DecSpecial::Finite || v.is_zero() {
        if v.is_zero() {
            return DecValue {
                special: DecSpecial::Finite,
                sign: v.sign,
                coeff: num_bigint::BigInt::from(0),
                exp: 0,
            };
        }
        return v.clone();
    }
    let mut coeff = v.coeff.clone();
    let mut exp = v.exp;
    let ten = num_bigint::BigInt::from(10);
    while &coeff % &ten == num_bigint::BigInt::from(0) && coeff != num_bigint::BigInt::from(0) {
        coeff /= &ten;
        exp += 1;
    }
    DecValue {
        special: DecSpecial::Finite,
        sign: v.sign,
        coeff,
        exp,
    }
}

pub fn get_decimal_type() -> PyObjectRef {
    let existing = DECIMAL_TYPE.with(|c| c.borrow().clone());
    if let Some(t) = existing {
        return t;
    }
    let typ = build_decimal_type();
    DECIMAL_TYPE.with(|c| {
        *c.borrow_mut() = Some(typ.clone());
    });
    typ
}

