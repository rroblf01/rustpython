// Split from src/object/ops_binary.rs — pow operations (`**`) and complex helpers.
use super::*;
use crate::object::*;
use num_bigint::{BigInt, Sign};
use num_traits::One;

/// Shared `float ** float` (also used for the mixed int/float cases)
/// helper for the `**` operator / `pow()` builtin — matches real CPython's
/// `float.__pow__`: `0.0 ** negative` raises `ZeroDivisionError` (NOT
/// `math.pow`'s own `ValueError: math domain error` — the two raise
/// DIFFERENT exception types for the same mathematical case), and a
/// genuine overflow (both operands finite, result isn't) raises
/// `OverflowError` instead of silently returning `inf`. Found via
/// CPython's own `test_math.py`/operator-level `pow()` overflow checks.
fn py_pow_float(x: f64, y: f64) -> PyResult<PyObjectRef> {
    // Only a FINITE negative exponent is an error — `0.0 ** -inf`
    // legitimately diverges to `inf` (same IEEE-754 `pow()` semantics as
    // `math.pow`'s analogous domain-error check).
    if x == 0.0 && y < 0.0 && y.is_finite() {
        return Err(PyError::ZeroDivisionError(
            "0.0 cannot be raised to a negative power".to_string(),
        ));
    }
    // A finite negative base with a NON-INTEGER exponent defers to complex
    // pow (CPython: (-2.0)**0.5 is complex ~ (8.66e-17+1.41j)). -INF stays
    // on the real path — IEEE powf(-inf, -0.5) == 0.0, (-inf)**0.5 == +inf,
    // which the complex path would wrongly turn into a signed zero/NaN.
    if x < 0.0 && x.is_finite() && y.fract() != 0.0 && y.is_finite() {
        let r = (-x).powf(y);
        let theta = y * std::f64::consts::PI;
        return Ok(PyObjectRef::imm(PyObject::Complex(
            r * theta.cos(),
            r * theta.sin(),
        )));
    }
    let result = x.powf(y);
    if result.is_infinite() && x.is_finite() && y.is_finite() {
        return Err(PyError::overflow_error(
            "(34, 'Numerical result out of range')",
        ));
    }
    Ok(py_float(result))
}

pub fn py_pow(a: &PyObjectRef, b: &PyObjectRef) -> PyResult<PyObjectRef> {
    if let (Some(ai), Some(bi)) = (a.as_i64(), b.as_i64()) {
        if bi < 0 {
            return py_pow_float(ai as f64, bi as f64);
        }
        if bi == 0 {
            return Ok(py_int(1));
        }
        if bi == 1 {
            return Ok(py_int(ai));
        }
        // Real CPython promotes to an arbitrary-precision int the instant
        // a computation would overflow, regardless of how "small" the
        // exponent looks. The previous "use BigInt only when bi > 63"
        // heuristic was unsound two ways: (1) the boundary itself was off
        // by one — `2**63` (exponent exactly 63) fell into the FAST i64
        // path below and silently wrapped via `wrapping_mul` to
        // `i64::MIN` instead of the correct `9223372036854775808`; (2) an
        // exponent under 63 can still overflow i64 if the BASE is large
        // enough (`3**40` already exceeds i64::MAX). Confirmed via
        // CPython's own `test_math.testIsqrt`, which fed `2**e` for `e`
        // up to 200 into `isqrt` and got a spurious `ValueError:
        // isqrt() argument must be nonnegative` from the wrapped-negative
        // `2**63`. Using checked arithmetic and falling back to BigInt on
        // ANY overflow (not just large exponents) fixes both.
        if bi <= u32::MAX as i64 {
            if let Some(result) = ai.checked_pow(bi as u32) {
                return Ok(py_int(result));
            }
        }
        let big_a = BigInt::from(ai);
        let result = big_a.pow(bi as u32);
        return Ok(py_int(result));
    }
    if let Some(r) = try_dunder_binop(a, b, "__pow__")? {
        return Ok(r);
    }
    if let Some(r) = try_dunder_binop(b, a, "__rpow__")? {
        return Ok(r);
    }
    let a_obj = a.borrow();
    let b_obj = b.borrow();
    match (&*a_obj, &*b_obj) {
        (PyObject::Int(a), PyObject::Int(b)) => {
            if let Some(exp) = b.to_usize() {
                let result = a.pow(exp as u32);
                Ok(py_int(result))
            } else if b.is_zero() {
                Ok(py_int(BigInt::one()))
            } else if b.sign() == Sign::Minus {
                // For now, return float
                let f = a.to_f64().unwrap().powf(b.to_f64().unwrap());
                Ok(py_float(f))
            } else {
                Err(PyError::value_error("int too large to convert to int"))
            }
        }
        (PyObject::Float(a), PyObject::Float(b)) => py_pow_float(*a, *b),
        (PyObject::Int(a), PyObject::Float(b)) => py_pow_float(a.to_f64().unwrap(), *b),
        (PyObject::Float(a), PyObject::Int(b)) => py_pow_float(*a, b.to_f64().unwrap()),
        // `complex ** (int|float|complex)` and `(int|float) ** complex` were
        // entirely unhandled — found via CPython's own `test_complex.py`.
        // Uses exact repeated-squaring for a real integer exponent (matching
        // real CPython's own fast path, and precise for e.g. `(1+2j)**2`
        // rather than accumulating log/exp floating-point error), falling
        // back to the general `z**w = exp(w * ln z)` polar-form identity
        // otherwise (fractional or complex exponents).
        _ if as_complex_parts(&a_obj).is_some()
            && as_complex_parts(&b_obj).is_some()
            && (matches!(&*a_obj, PyObject::Complex(_, _))
                || matches!(&*b_obj, PyObject::Complex(_, _))) =>
        {
            let (are, aim) = as_complex_parts(&a_obj).unwrap();
            let (bre, bim) = as_complex_parts(&b_obj).unwrap();
            complex_pow(are, aim, bre, bim)
        }
        _ => Err(PyError::type_error(format!(
            "unsupported operand type(s) for **: '{}' and '{}'",
            a_obj.type_name(),
            b_obj.type_name()
        ))),
    }
}

fn complex_mul(are: f64, aim: f64, bre: f64, bim: f64) -> (f64, f64) {
    (are * bre - aim * bim, are * bim + aim * bre)
}

fn complex_pow_int(are: f64, aim: f64, n: i64) -> (f64, f64) {
    let neg = n < 0;
    let mut n = n.unsigned_abs();
    let mut result = (1.0f64, 0.0f64);
    let mut base = (are, aim);
    while n > 0 {
        if n & 1 == 1 {
            result = complex_mul(result.0, result.1, base.0, base.1);
        }
        base = complex_mul(base.0, base.1, base.0, base.1);
        n >>= 1;
    }
    if neg {
        let denom = result.0 * result.0 + result.1 * result.1;
        (result.0 / denom, -result.1 / denom)
    } else {
        result
    }
}

fn complex_pow(are: f64, aim: f64, bre: f64, bim: f64) -> PyResult<PyObjectRef> {
    let base_zero = are == 0.0 && aim == 0.0;
    // A non-finite result computed from FINITE inputs is an overflow
    // (repeated squaring / exp(w*ln z) spill to inf/nan); NaN inputs legitimately
    // propagate NaN instead.
    let inputs_finite = are.is_finite() && aim.is_finite() && bre.is_finite() && bim.is_finite();
    let overflow = |re: f64, im: f64| -> PyResult<PyObjectRef> {
        if inputs_finite && (!re.is_finite() || !im.is_finite()) {
            Err(PyError::overflow_error("complex exponentiation"))
        } else {
            Ok(PyObjectRef::imm(PyObject::Complex(re, im)))
        }
    };
    if bim == 0.0 && bre.fract() == 0.0 && bre.abs() < 1e15 {
        // Integer exponent: 0 to a negative power is an error.
        if base_zero && bre < 0.0 {
            return Err(PyError::zero_division());
        }
        let (re, im) = complex_pow_int(are, aim, bre as i64);
        return overflow(re, im);
    }
    // General case: z^w = exp(w * ln z), ln z = ln|z| + i*arg(z). A zero
    // base raised to a negative or complex power is a domain error; 0 to a
    // positive real power is 0; 0 to zero power is 1.
    if base_zero {
        if bim != 0.0 || bre < 0.0 {
            return Err(PyError::zero_division());
        }
        if bre == 0.0 {
            return Ok(PyObjectRef::imm(PyObject::Complex(1.0, 0.0)));
        }
        return Ok(PyObjectRef::imm(PyObject::Complex(0.0, 0.0)));
    }
    let r = (are * are + aim * aim).sqrt();
    let theta = aim.atan2(are);
    let (ere, eim) = complex_mul(bre, bim, r.ln(), theta);
    let exp_re = ere.exp();
    let (re, im) = (exp_re * eim.cos(), exp_re * eim.sin());
    overflow(re, im)
}
