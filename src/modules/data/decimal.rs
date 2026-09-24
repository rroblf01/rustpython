use crate::modules::data::decimal_types::build_decimal_type;
use crate::modules::data::fractions::{frac_instance_num_den, frac_normalize, frac_to_f64};
use crate::object::*;
use num_bigint::{BigInt, Sign};
use num_traits::{One, Signed, ToPrimitive, Zero};
use std::collections::HashMap;
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

/// Parsed `Decimal.__format__` mini-language state.
struct DecFmtSpec {
    fill: char,
    align: char,
    sign: char,
    alt: bool,
    width: usize,
    grouping: Option<char>,
    precision: Option<usize>,
    ty: char, // '\0' = none, else one of e E f F g G n %
}

fn parse_dec_fmt_spec(spec: &str) -> Result<DecFmtSpec, PyError> {
    let invalid = || PyError::value_error("invalid format string".to_string());
    let chars: Vec<char> = spec.chars().collect();
    let len = chars.len();
    let mut idx = 0;
    let mut fill = ' ';
    let mut align = '>';
    let mut explicit_align = false;
    if idx + 1 < len && matches!(chars[idx + 1], '<' | '>' | '=' | '^') {
        fill = chars[idx];
        align = chars[idx + 1];
        idx += 2;
        explicit_align = true;
    } else if idx < len && matches!(chars[idx], '<' | '>' | '=' | '^') {
        align = chars[idx];
        idx += 1;
        explicit_align = true;
    }
    let mut sign = '-';
    if idx < len && matches!(chars[idx], '+' | '-' | ' ') {
        sign = chars[idx];
        idx += 1;
    }
    if idx < len && chars[idx] == 'z' {
        idx += 1;
    }
    let mut alt = false;
    if idx < len && chars[idx] == '#' {
        alt = true;
        idx += 1;
    }
    if idx < len && chars[idx] == '0' {
        // A `0` after an explicit alignment is invalid (`>010`);
        // otherwise it means zero-padding (`010`).
        if explicit_align {
            return Err(invalid());
        }
        fill = '0';
        align = '=';
        idx += 1;
    }
    let mut width = 0usize;
    while idx < len && chars[idx].is_ascii_digit() {
        width = width * 10 + (chars[idx] as usize - '0' as usize);
        idx += 1;
    }
    let mut grouping = None;
    if idx < len && (chars[idx] == ',' || chars[idx] == '_') {
        let first = chars[idx];
        grouping = Some(first);
        idx += 1;
        if idx < len && (chars[idx] == ',' || chars[idx] == '_') {
            return Err(PyError::value_error(format!(
                "Cannot specify '{}' with '{}'.",
                first, chars[idx]
            )));
        }
    }
    let mut precision = None;
    if idx < len && chars[idx] == '.' {
        idx += 1;
        let pstart = idx;
        let mut p = 0usize;
        while idx < len && chars[idx].is_ascii_digit() {
            p = p * 10 + (chars[idx] as usize - '0' as usize);
            idx += 1;
        }
        if idx == pstart {
            return Err(invalid()); // `.f` — precision needs digits
        }
        precision = Some(p);
    }
    let mut ty = '\0';
    if idx < len {
        ty = chars[idx];
        if !matches!(ty, 'e' | 'E' | 'f' | 'F' | 'g' | 'G' | 'n' | '%') {
            return Err(invalid());
        }
        idx += 1;
    }
    if idx != len {
        return Err(invalid());
    }
    Ok(DecFmtSpec {
        fill,
        align,
        sign,
        alt,
        width,
        grouping,
        precision,
        ty,
    })
}

/// Round `coeff` down by `drop` decimal digits (ROUND_HALF_EVEN).
fn drop_digits_half_even(coeff: &num_bigint::BigInt, drop: usize) -> num_bigint::BigInt {
    use num_bigint::BigInt;
    if drop == 0 {
        return coeff.clone();
    }
    let divisor = ten_pow(drop as i64);
    let q = coeff / &divisor;
    let r = coeff % &divisor;
    let twice = &r * BigInt::from(2);
    use std::cmp::Ordering;
    let up = match twice.cmp(&divisor) {
        Ordering::Greater => true,
        Ordering::Less => false,
        Ordering::Equal => (&q % 2) != BigInt::from(0),
    };
    if up {
        q + 1
    } else {
        q
    }
}

/// Fixed-point digits for `coeff * 10^exp` with exactly `frac`
/// fraction digits: returns (integer_digits, frac_digits).
fn dec_fixed_parts(coeff: &num_bigint::BigInt, exp: i64, frac: usize) -> (String, String) {
    let mut c = coeff.clone();
    if -exp > frac as i64 {
        c = drop_digits_half_even(&c, (-exp - frac as i64) as usize);
    } else if -exp < frac as i64 {
        c = c * ten_pow(frac as i64 + exp);
    }
    let s = c.to_string();
    if frac == 0 {
        return (s, String::new());
    }
    if s.len() <= frac {
        ("0".to_string(), format!("{:0>width$}", s, width = frac))
    } else {
        let (i, f) = s.split_at(s.len() - frac);
        (i.to_string(), f.to_string())
    }
}

/// Group the leading integer run of a rendered number in threes
/// (`,`/`_`), leaving sign, fraction and exponent untouched.
fn group_leading_int(s: &str, sep: char) -> String {
    let bytes = s.as_bytes();
    let mut i = 0;
    if i < bytes.len() && (bytes[i] == b'-' || bytes[i] == b'+' || bytes[i] == b' ') {
        i += 1;
    }
    let int_start = i;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i - int_start <= 3 {
        return s.to_string();
    }
    let mut out = String::new();
    out.push_str(&s[..int_start]);
    out.push_str(&group_dec_digits(&s[int_start..i], sep));
    out.push_str(&s[i..]);
    out
}

/// Group an integer-digit string in threes (`,`/`_`).
fn group_dec_digits(intpart: &str, sep: char) -> String {
    if intpart.len() <= 3 {
        return intpart.to_string();
    }
    let mut out = String::new();
    let rem = intpart.len() % 3;
    for (i, ch) in intpart.chars().enumerate() {
        if i > 0 && (i - rem) % 3 == 0 {
            out.push(sep);
        }
        out.push(ch);
    }
    out
}

fn dec_exp_str(adj: i64, upper: bool) -> String {
    format!(
        "{}{}{}",
        if upper { 'E' } else { 'e' },
        if adj < 0 { '-' } else { '+' },
        adj.abs()
    )
}

/// Pad `num` (already signed) to `width` with fill/align.
fn apply_dec_width(num: &str, p: &DecFmtSpec) -> String {
    let nchars: usize = num.chars().count();
    if nchars >= p.width {
        return num.to_string();
    }
    let pad = p.width - nchars;
    let padstr: String = std::iter::repeat(p.fill).take(pad).collect();
    match p.align {
        '<' => format!("{}{}", num, padstr),
        '^' => {
            let left = pad / 2;
            let right = pad - left;
            format!(
                "{}{}{}",
                &padstr[..left.min(padstr.len())],
                num,
                &padstr[..right.min(padstr.len())]
            )
        }
        '=' => {
            // Pad after a leading sign, if any.
            let mut chars = num.chars();
            let first = chars.next().unwrap();
            if matches!(first, '+' | '-' | ' ') {
                format!("{}{}{}", first, padstr, chars.collect::<String>())
            } else {
                format!("{}{}", padstr, num)
            }
        }
        _ => format!("{}{}", padstr, num),
    }
}

/// General float-style rendering shared by `g`/`G`/`n` and by a
/// precision with no type (which behaves like `G`; mirrors CPython:
/// `format(D('1200'), '.2')` is `'1.2E+3'`). `prec_opt=None` is
/// handled by the caller (digits as-is).
fn render_dec_general(
    coeff: &num_bigint::BigInt,
    exp: i64,
    is_zero: bool,
    prec_opt: Option<usize>,
    upper: bool,
    alt: bool,
    grouping: Option<char>,
) -> String {
    let mut prec = prec_opt.unwrap_or(1);
    if prec == 0 {
        prec = 1;
    }
    let mut body = String::new();
    if is_zero {
        body.push('0');
        if alt {
            body.push('.');
        }
        return body;
    }
    // Round to `prec` significant digits, then select fixed vs
    // scientific on the rounded exponent.
    let d = digit_count(coeff);
    let mut q = coeff.clone();
    let mut dd = d;
    // Exponent of `q` (it gains the dropped orders).
    let mut qexp = exp;
    if d > prec {
        q = drop_digits_half_even(coeff, d - prec);
        qexp = exp + (d - prec) as i64;
        if digit_count(&q) > prec {
            q = &q / 10;
            dd = d + 1;
            qexp += 1;
        } else {
            dd = d;
        }
    }
    let exp10 = dd as i64 - 1 + exp;
    if -4 <= exp10 && exp10 < prec as i64 {
        let frac = (prec as i64 - 1 - exp10).max(0) as usize;
        let (mut ip, mut fp) = dec_fixed_parts(&q, qexp, frac);
        if !alt {
            while fp.ends_with('0') {
                fp.pop();
            }
        }
        if let Some(g) = grouping {
            ip = group_dec_digits(&ip, g);
        }
        body.push_str(&ip);
        if !fp.is_empty() || alt {
            body.push('.');
            body.push_str(&fp);
        }
    } else {
        let mut qs = q.to_string();
        // Decimals originally present (rounded zeros stay: `.2g` on
        // 99.9 is `1.0e+2`); only padding-added zeros strip (`.3g`
        // on 5 is `5`, not `5.00`).
        let orig_decimals = qs.len().saturating_sub(1);
        while qs.len() < prec {
            qs.push('0');
        }
        let (first, rest) = qs.split_at(1);
        body.push_str(first);
        let mut fracpart = rest[..prec.min(rest.len())].to_string();
        for _ in rest.len()..prec.saturating_sub(1) {
            fracpart.push('0');
        }
        if !alt {
            while fracpart.len() > orig_decimals.min(fracpart.len()) && fracpart.ends_with('0') {
                fracpart.pop();
            }
        }
        if !fracpart.is_empty() || alt {
            body.push('.');
            body.push_str(&fracpart);
        }
        body.push_str(&dec_exp_str(dd as i64 - 1 + exp, upper));
    }
    body
}

/// `Decimal.__format__` body (mirrors CPython: exact decimal
/// arithmetic with ROUND_HALF_EVEN, never float).
pub fn format_decimal_spec(v: &DecValue, spec: &str) -> Result<String, PyError> {
    use num_traits::Zero;
    let p = parse_dec_fmt_spec(spec)?;
    if v.special != DecSpecial::Finite {
        let mut s = format_decvalue(v);
        if !v.sign {
            if p.sign == '+' {
                s = format!("+{}", s);
            } else if p.sign == ' ' {
                s = format!(" {}", s);
            }
        }
        return Ok(apply_dec_width(&s, &p));
    }
    let is_zero = v.coeff.is_zero();
    // `%` scales by 100 and formats fixed (mirrors CPython).
    let (coeff, exp, mut ty) = if p.ty == '%' {
        (v.coeff.clone(), v.exp + 2, 'f')
    } else {
        (v.coeff.clone(), v.exp, p.ty)
    };
    if ty == 'n' {
        ty = 'g';
    }
    // Render the unsigned body, then sign, grouping, width.
    let mut body = String::new();
    if ty == '\0' {
        if p.precision.is_some() {
            // Precision with no type behaves like `G` (uppercase;
            // mirrors CPython: `format(D('1200'), '.2')` is `'1.2E+3'`).
            body.push_str(&render_dec_general(
                &coeff,
                exp,
                is_zero,
                p.precision,
                true,
                p.alt,
                p.grouping,
            ));
        } else {
            // No type, no precision: digits as-is (mirrors CPython:
            // `1E+10`, trailing zeros kept). `#` forces a point.
            // A grouping flag still groups (`format(D('1234567.8'),
            // ',')`).
            let mut s = format_decvalue(&DecValue {
                special: DecSpecial::Finite,
                sign: false,
                coeff: coeff.clone(),
                exp,
            });
            if let Some(g) = p.grouping {
                s = group_leading_int(&s, g);
            }
            body.push_str(&s);
            if p.alt && !body.contains('.') && !body.contains('e') && !body.contains('E') {
                body.push('.');
            }
        }
    } else if ty == 'f' || ty == 'F' {
        let frac = p.precision.unwrap_or_else(|| (-exp).max(0) as usize);
        let (mut ip, fp) = dec_fixed_parts(&coeff, exp, frac);
        if let Some(g) = p.grouping {
            ip = group_dec_digits(&ip, g);
        }
        body.push_str(&ip);
        if frac > 0 {
            body.push('.');
            body.push_str(&fp);
        } else if p.alt {
            body.push('.');
        }
    } else if ty == 'e' || ty == 'E' {
        let upper = ty == 'E';
        if is_zero {
            // CPython quirk: zero renders `precision + exp` as the
            // exponent (`format(D('0'), '.2e')` is `'0.00e+2'`).
            let prec = p.precision.unwrap_or_else(|| {
                let d = digit_count(&coeff);
                if d == 0 {
                    0
                } else {
                    d - 1
                }
            });
            body.push('0');
            if prec > 0 || p.alt {
                body.push('.');
                for _ in 0..prec {
                    body.push('0');
                }
            }
            body.push_str(&dec_exp_str(prec as i64 + exp, upper));
        } else {
            let prec = p.precision.unwrap_or_else(|| digit_count(&coeff) - 1);
            let d = digit_count(&coeff);
            let mut q = coeff.clone();
            let mut dd = d;
            if d > prec + 1 {
                q = drop_digits_half_even(&coeff, d - prec - 1);
                if digit_count(&q) > prec + 1 {
                    q = &q / 10;
                    dd = d + 1;
                } else {
                    dd = d;
                }
            }
            let mut qs = q.to_string();
            while qs.len() < prec + 1 {
                qs.push('0');
            }
            let (first, rest) = qs.split_at(1);
            body.push_str(first);
            if prec > 0 || p.alt {
                body.push('.');
                body.push_str(&rest[..prec.min(rest.len())]);
                for _ in rest.len()..prec {
                    body.push('0');
                }
            }
            body.push_str(&dec_exp_str(dd as i64 - 1 + exp, upper));
        }
    } else {
        // g/G (n folds in): precision or all digits.
        let upper = ty == 'G';
        if p.precision.is_none() {
            // No precision: digits as-is, with `e` normalized to the
            // type's case (mirrors CPython: `format(D('1E+10'), 'g')`
            // is `'1e+10'`).
            body.push_str(&format_decvalue(&DecValue {
                special: DecSpecial::Finite,
                sign: false,
                coeff: coeff.clone(),
                exp,
            }));
            if upper {
                body = body.replace('e', "E");
            } else {
                body = body.replace('E', "e");
            }
        } else {
            body.push_str(&render_dec_general(
                &coeff,
                exp,
                is_zero,
                p.precision,
                upper,
                p.alt,
                p.grouping,
            ));
        }
    }
    if p.ty == '%' {
        body.push('%');
    }
    // Sign.
    let mut signed = String::new();
    if v.sign {
        signed.push('-');
    } else if p.sign == '+' {
        signed.push('+');
    } else if p.sign == ' ' {
        signed.push(' ');
    }
    signed.push_str(&body);
    Ok(apply_dec_width(&signed, &p))
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
    // The live current context object is authoritative; the tuple cache is
    // only a fallback for the window before it exists.
    let ctx = crate::modules::data::decimal_ctx::current_context();
    let prec = crate::modules::data::decimal_ctx::ctx_prec(&ctx);
    let rounding = crate::modules::data::decimal_ctx::ctx_rounding(&ctx);
    (prec, rounding)
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

/// Round `v` through the CURRENT context object, recording signals and
/// honouring traps. Used by the operators (`+`, `-`, `*`, `/`, `//`, `%`,
/// `**`) so `ctx.flags[Inexact]` and trapped signals behave like CPython's.
pub fn round_to_current_context(v: DecValue) -> PyResult<DecValue> {
    let ctx = crate::modules::data::decimal_ctx::current_context();
    crate::modules::data::decimal_ctx::apply_context(&ctx, v)
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
            // `(+Inf) + (-Inf)` signals InvalidOperation: the flag is always
            // recorded, the exception is raised only when the current
            // context TRAPS it (CPython). The old code keyed this off a
            // `BasicContext` marker instead, so `ctx.traps[InvalidOperation]
            // = True` had no effect.
            let ctx = crate::modules::data::decimal_ctx::current_context();
            if current_is_basic()
                || crate::modules::data::decimal_ctx::signal_trapped(&ctx, "InvalidOperation")
            {
                crate::modules::data::decimal_ctx::write_signal(
                    &ctx,
                    "flags",
                    "InvalidOperation",
                    true,
                );
                return Err(crate::modules::data::decimal_ctx::decimal_signal_error(
                    "InvalidOperation",
                    "[<class 'decimal.Overflow'>]",
                ));
            }
            crate::modules::data::decimal_ctx::write_signal(
                &ctx,
                "flags",
                "InvalidOperation",
                true,
            );
            return Ok(DecValue::nan());
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
    round_to_current_context(result)
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
    round_to_current_context(result)
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
        return round_to_current_context(DecValue {
            special: DecSpecial::Finite,
            sign,
            coeff: num_bigint::BigInt::from(0),
            exp: a.exp - b.exp,
        });
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
