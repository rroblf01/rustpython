use crate::object::*;
use std::collections::HashMap;

fn arg_complex(args: &[PyObjectRef], idx: usize, fname: &str) -> Result<(f64, f64), PyError> {
    let v = args
        .get(idx)
        .ok_or_else(|| PyError::type_error(format!("{fname}() missing argument")))?;
    let b = v.borrow();
    if let Some(p) = as_complex_parts(&b) {
        return Ok(p);
    }
    drop(b);
    // Number protocol: __complex__, __float__, __index__ (in that order;
    // notably NOT __int__ — test_cmath::test_user_object requires TypeError
    // for __int__-only classes). Call errors propagate (e.g. a raising
    // __complex__); missing methods fall through to the next protocol.
    // One level only (results must be real numbers, not recurse). Pass
    // `self` explicitly unless already a bound method.
    let call_protocol = |m: &PyObjectRef| -> PyResult<PyObjectRef> {
        let bound = matches!(&*m.borrow(), PyObject::BoundMethod { .. });
        let call_args: Vec<PyObjectRef> = if bound { vec![] } else { vec![v.clone()] };
        crate::object::builtin_call(m, &call_args)
    };
    if let Ok(m) = v.borrow().get_attribute("__complex__") {
        let r = call_protocol(&m)?;
        let rb = r.borrow();
        // CPython requires __complex__ to return exactly a complex
        // (int/float/str results are TypeError, not coerced).
        if let PyObject::Complex(re, im) = &*rb {
            return Ok((*re, *im));
        }
        return Err(PyError::type_error(format!(
            "{fname}() argument must be a number"
        )));
    }
    if let Ok(m) = v.borrow().get_attribute("__float__") {
        let r = call_protocol(&m)?;
        if let Some(f) = r.as_f64() {
            return Ok((f, 0.0));
        }
        return Err(PyError::type_error(format!(
            "{fname}() argument must be a number"
        )));
    }
    if let Ok(m) = v.borrow().get_attribute("__index__") {
        let r = call_protocol(&m)?;
        if let Some(i) = r.as_i64() {
            return Ok((i as f64, 0.0));
        }
        return Err(PyError::type_error(format!(
            "{fname}() argument must be a number"
        )));
    }
    Err(PyError::type_error(format!(
        "{fname}() argument must be a number"
    )))
}

fn py_complex_or_float(re: f64, im: f64) -> PyObjectRef {
    PyObjectRef::imm(PyObject::Complex(re, im))
}

/// Real part of `ln(z)` for complex `z` — `ln(|z|)`. Shared by `log`/`log10`
/// and by `atan`/`atanh`'s formulas below.
fn c_abs(re: f64, im: f64) -> f64 {
    re.hypot(im)
}

fn c_phase(re: f64, im: f64) -> f64 {
    im.atan2(re)
}

fn c_ln(re: f64, im: f64) -> (f64, f64) {
    // Scale to avoid spurious overflow: hypot(1.6e308, 1.5e308) is inf,
    // but log|z| ~ 710 is finite (test_cmath log0103). Conversely scale
    // UP for subnormals to keep full precision (log0121).
    let r = if re.abs() < f64::MIN_POSITIVE
        && im.abs() < f64::MIN_POSITIVE
        && (re != 0.0 || im != 0.0)
    {
        const UP: f64 = 4503599627370496.0; // 2^52
        (re * UP).hypot(im * UP).ln() - 52.0 * std::f64::consts::LN_2
    } else {
        let h = re.hypot(im);
        if h.is_infinite() && re.is_finite() && im.is_finite() {
            (re / 2.0).hypot(im / 2.0).ln() + std::f64::consts::LN_2
        } else {
            h.ln()
        }
    };
    (r, c_phase(re, im))
}

const EXP_TABLE: SpecialTable = [
    [(0.0, 0.0), U, (0.0, -0.0), (0.0, 0.0), U, (0.0, 0.0), (0.0, 0.0)],
    [(NAN, NAN), U, U, U, U, (NAN, NAN), (NAN, NAN)],
    [(NAN, NAN), U, (1.0, -0.0), (1.0, 0.0), U, (NAN, NAN), (NAN, NAN)],
    [(NAN, NAN), U, (1.0, -0.0), (1.0, 0.0), U, (NAN, NAN), (NAN, NAN)],
    [(NAN, NAN), U, U, U, U, (NAN, NAN), (NAN, NAN)],
    [(INF, NAN), U, (INF, -0.0), (INF, 0.0), U, (INF, NAN), (INF, NAN)],
    [(NAN, NAN), (NAN, NAN), (NAN, -0.0), (NAN, 0.0), (NAN, NAN), (NAN, NAN), (NAN, NAN)],
];

fn c_exp(re: f64, im: f64) -> PyResult<(f64, f64)> {
    // CPython sets EDOM (→ ValueError) when imag is infinite and real is
    // finite or +inf.
    if im.is_infinite() && (re.is_finite() || re == INF) {
        return Err(PyError::value_error("math domain error"));
    }
    if !re.is_finite() || !im.is_finite() {
        // Custom (non-macro) branch: infinite real with finite nonzero
        // imag uses signed zeros from cos/sin.
        if re.is_infinite() && im.is_finite() && im != 0.0 {
            if re > 0.0 {
                return Ok((INF.copysign(im.cos()), INF.copysign(im.sin())));
            } else {
                return Ok((0.0f64.copysign(im.cos()), 0.0f64.copysign(im.sin())));
            }
        }
        if let Some(v) = special_lookup(&EXP_TABLE, re, im) {
            return Ok(v);
        }
    }
    // Scale to avoid spurious intermediate overflow: exp(710) is inf,
    // but exp(709)*e is finite (test_cmath exp0030).
    let (r, i) = if re > f64::MAX.ln() {
        let l = (re - 1.0).exp();
        (
            l * im.cos() * std::f64::consts::E,
            l * im.sin() * std::f64::consts::E,
        )
    } else {
        let mag = re.exp();
        (mag * im.cos(), mag * im.sin())
    };
    if r.is_infinite() || i.is_infinite() {
        return Err(PyError::overflow_error("math range error"));
    }
    Ok((r, i))
}

fn c_mul(a: (f64, f64), b: (f64, f64)) -> (f64, f64) {
    (a.0 * b.0 - a.1 * b.1, a.0 * b.1 + a.1 * b.0)
}

fn c_div(a: (f64, f64), b: (f64, f64)) -> (f64, f64) {
    // Smith's algorithm (CPython _Py_c_quot), matching complex.__truediv__:
    // avoids overflow and preserves signed zeros (e.g. log(1, 0.5) real
    // must be -0.0, which the naive sum-of-squares form loses).
    if b.0.abs() >= b.1.abs() {
        let ratio = b.1 / b.0;
        let denom = b.0 + b.1 * ratio;
        (
            (a.0 + a.1 * ratio) / denom,
            (a.1 - a.0 * ratio) / denom,
        )
    } else {
        let ratio = b.0 / b.1;
        let denom = b.0 * ratio + b.1;
        (
            (a.0 * ratio + a.1) / denom,
            (a.1 * ratio - a.0) / denom,
        )
    }
}

fn c_add(a: (f64, f64), b: (f64, f64)) -> (f64, f64) {
    (a.0 + b.0, a.1 + b.1)
}

fn c_sub(a: (f64, f64), b: (f64, f64)) -> (f64, f64) {
    (a.0 - b.0, a.1 - b.1)
}

const TANH_TABLE: SpecialTable = [
    [(-1.0, 0.0), U, (-1.0, -0.0), (-1.0, 0.0), U, (-1.0, 0.0), (-1.0, 0.0)],
    [(NAN, NAN), U, U, U, U, (NAN, NAN), (NAN, NAN)],
    [(-0.0, NAN), U, (-0.0, -0.0), (-0.0, 0.0), U, (-0.0, NAN), (-0.0, NAN)],
    [(0.0, NAN), U, (0.0, -0.0), (0.0, 0.0), U, (0.0, NAN), (0.0, NAN)],
    [(NAN, NAN), U, U, U, U, (NAN, NAN), (NAN, NAN)],
    [(1.0, 0.0), U, (1.0, -0.0), (1.0, 0.0), U, (1.0, 0.0), (1.0, 0.0)],
    [(NAN, NAN), (NAN, NAN), (NAN, -0.0), (NAN, 0.0), (NAN, NAN), (NAN, NAN), (NAN, NAN)],
];

fn c_tanh(re: f64, im: f64) -> PyResult<(f64, f64)> {
    if !re.is_finite() || !im.is_finite() {
        if re.is_infinite() && im.is_finite() && im != 0.0 {
            let s = 2.0 * im.sin() * im.cos();
            if re > 0.0 {
                return Ok((1.0, 0.0f64.copysign(s)));
            } else {
                return Ok((-1.0, 0.0f64.copysign(s)));
            }
        }
        if im.is_infinite() && re.is_finite() {
            return Err(PyError::value_error("math domain error"));
        }
        if let Some(v) = special_lookup(&TANH_TABLE, re, im) {
            return Ok(v);
        }
    }
    if re.abs() > f64::MAX.ln() {
        let r = 1.0f64.copysign(re);
        let i = 4.0 * im.sin() * im.cos() * (-2.0 * re.abs()).exp();
        return Ok((r, i));
    }
    let tx = re.tanh();
    let ty = im.tan();
    let cx = 1.0 / re.cosh();
    let txty = tx * ty;
    let denom = 1.0 + txty * txty;
    Ok((tx * (1.0 + ty * ty) / denom, ((ty / denom) * cx) * cx))
}

// ── CPython-exact inverse functions (Modules/cmathmodule.c) ──────────────
// The naive textbook formulas above get branch cuts and signed zeros
// wrong (e.g. acos(0j) must be pi/2-0j, atan needs atanh's edge cases).
// Ported directly, including special-value tables for non-finite inputs
// and large-argument overflow guards.

// special_type codes (match CPython's enum order for table indexing).
const ST_NINF: usize = 0;
const ST_NEG: usize = 1;
const ST_NZERO: usize = 2;
const ST_PZERO: usize = 3;
const ST_POS: usize = 4;
const ST_PINF: usize = 5;
const ST_NAN: usize = 6;

fn special_type(d: f64) -> usize {
    // NB: CPython tests `copysign(1., d) == 1.` — magnitude of 1.0 with
    // d's sign. `d.copysign(1.0)` is the reverse (d's magnitude, always
    // positive sign) and misclassifies everything.
    if d.is_finite() {
        if d != 0.0 {
            if 1.0f64.copysign(d) == 1.0 {
                ST_POS
            } else {
                ST_NEG
            }
        } else if 1.0f64.copysign(d) == 1.0 {
            ST_PZERO
        } else {
            ST_NZERO
        }
    } else if d.is_nan() {
        ST_NAN
    } else if 1.0f64.copysign(d) == 1.0 {
        ST_PINF
    } else {
        ST_NINF
    }
}

const P: f64 = std::f64::consts::PI;
const P12: f64 = std::f64::consts::FRAC_PI_2;
const P14: f64 = std::f64::consts::FRAC_PI_4;
const P34: f64 = 3.0 * std::f64::consts::FRAC_PI_4;
const INF: f64 = f64::INFINITY;
const NAN: f64 = f64::NAN;
/// Unreachable placeholder (CPython's `U`): only finite rows/cols that the
/// `!isfinite` gate never selects. Never read; NaN marks bugs loudly.
const U: (f64, f64) = (NAN, NAN);

type SpecialTable = [[(f64, f64); 7]; 7];

fn special_lookup(table: &SpecialTable, re: f64, im: f64) -> Option<(f64, f64)> {
    if !re.is_finite() || !im.is_finite() {
        Some(table[special_type(re)][special_type(im)])
    } else {
        None
    }
}

const ACOS_TABLE: SpecialTable = [
    [(P34, INF), (P, INF), (P, INF), (P, -INF), (P, -INF), (P34, -INF), (NAN, INF)],
    [(P12, INF), U, U, U, U, (P12, -INF), (NAN, NAN)],
    [(P12, INF), U, (P12, 0.0), (P12, -0.0), U, (P12, -INF), (P12, NAN)],
    [(P12, INF), U, (P12, 0.0), (P12, -0.0), U, (P12, -INF), (P12, NAN)],
    [(P12, INF), U, U, U, U, (P12, -INF), (NAN, NAN)],
    [(P14, INF), (0.0, INF), (0.0, INF), (0.0, -INF), (0.0, -INF), (P14, -INF), (NAN, INF)],
    [(NAN, INF), (NAN, NAN), (NAN, NAN), (NAN, NAN), (NAN, NAN), (NAN, -INF), (NAN, NAN)],
];

const ASINH_TABLE: SpecialTable = [
    [(-INF, -P14), (-INF, -0.0), (-INF, -0.0), (-INF, 0.0), (-INF, 0.0), (-INF, P14), (-INF, NAN)],
    [(-INF, -P12), U, U, U, U, (-INF, P12), (NAN, NAN)],
    [(-INF, -P12), U, (-0.0, -0.0), (-0.0, 0.0), U, (-INF, P12), (NAN, NAN)],
    [(INF, -P12), U, (0.0, -0.0), (0.0, 0.0), U, (INF, P12), (NAN, NAN)],
    [(INF, -P12), U, U, U, U, (INF, P12), (NAN, NAN)],
    [(INF, -P14), (INF, -0.0), (INF, -0.0), (INF, 0.0), (INF, 0.0), (INF, P14), (INF, NAN)],
    [(INF, NAN), (NAN, NAN), (NAN, -0.0), (NAN, 0.0), (NAN, NAN), (INF, NAN), (NAN, NAN)],
];

const ATANH_TABLE: SpecialTable = [
    [(-0.0, -P12), (-0.0, -P12), (-0.0, -P12), (-0.0, P12), (-0.0, P12), (-0.0, P12), (-0.0, NAN)],
    [(-0.0, -P12), U, U, U, U, (-0.0, P12), (NAN, NAN)],
    [(-0.0, -P12), U, (-0.0, -0.0), (-0.0, 0.0), U, (-0.0, P12), (-0.0, NAN)],
    [(0.0, -P12), U, (0.0, -0.0), (0.0, 0.0), U, (0.0, P12), (0.0, NAN)],
    [(0.0, -P12), U, U, U, U, (0.0, P12), (NAN, NAN)],
    [(0.0, -P12), (0.0, -P12), (0.0, -P12), (0.0, P12), (0.0, P12), (0.0, P12), (0.0, NAN)],
    [(0.0, -P12), (NAN, NAN), (NAN, NAN), (NAN, NAN), (NAN, NAN), (0.0, P12), (NAN, NAN)],
];

const SQRT_TABLE: SpecialTable = [
    [(INF, -INF), (0.0, -INF), (0.0, -INF), (0.0, INF), (0.0, INF), (INF, INF), (NAN, INF)],
    [(INF, -INF), U, U, U, U, (INF, INF), (NAN, NAN)],
    [(INF, -INF), U, (0.0, -0.0), (0.0, 0.0), U, (INF, INF), (NAN, NAN)],
    [(INF, -INF), U, (0.0, -0.0), (0.0, 0.0), U, (INF, INF), (NAN, NAN)],
    [(INF, -INF), U, U, U, U, (INF, INF), (NAN, NAN)],
    [(INF, -INF), (INF, -0.0), (INF, -0.0), (INF, 0.0), (INF, 0.0), (INF, INF), (INF, NAN)],
    [(INF, -INF), (NAN, NAN), (NAN, NAN), (NAN, NAN), (NAN, NAN), (INF, INF), (NAN, NAN)],
];

const CM_LARGE_DOUBLE: f64 = f64::MAX / 4.0;
const CM_SQRT_DBL_MIN: f64 = 1.4916681462400413e-154; // sqrt(DBL_MIN)
#[inline]
fn cm_sqrt_large_double() -> f64 {
    (f64::MAX / 4.0).sqrt()
}
const CM_SCALE_UP: i32 = 53;
const CM_SCALE_DOWN: i32 = -27;

fn c_sqrt(re: f64, im: f64) -> (f64, f64) {
    if let Some(v) = special_lookup(&SQRT_TABLE, re, im) {
        return v;
    }
    if re == 0.0 && im == 0.0 {
        return (0.0, im);
    }
    let ax = re.abs();
    let ay = im.abs();
    // Subnormal inputs: rescale so hypot() keeps full precision.
    let s: f64 = if ax < f64::MIN_POSITIVE && ay < f64::MIN_POSITIVE {
        let ax_s = ax * 2f64.powi(CM_SCALE_UP);
        (ax_s + ax_s.hypot(ay * 2f64.powi(CM_SCALE_UP))).sqrt() * 2f64.powi(CM_SCALE_DOWN)
    } else {
        let ax8 = ax / 8.0;
        2.0 * (ax8 + ax8.hypot(ay / 8.0)).sqrt()
    };
    let d = ay / (2.0 * s);
    if re >= 0.0 {
        (s, d.copysign(im))
    } else {
        (d, s.copysign(im))
    }
}

const SINH_TABLE: SpecialTable = [
    [(INF, NAN), U, (-INF, -0.0), (-INF, 0.0), U, (INF, NAN), (INF, NAN)],
    [(NAN, NAN), U, U, U, U, (NAN, NAN), (NAN, NAN)],
    [(0.0, NAN), U, (-0.0, -0.0), (-0.0, 0.0), U, (0.0, NAN), (0.0, NAN)],
    [(0.0, NAN), U, (0.0, -0.0), (0.0, 0.0), U, (0.0, NAN), (0.0, NAN)],
    [(NAN, NAN), U, U, U, U, (NAN, NAN), (NAN, NAN)],
    [(INF, NAN), U, (INF, -0.0), (INF, 0.0), U, (INF, NAN), (INF, NAN)],
    [(NAN, NAN), (NAN, NAN), (NAN, -0.0), (NAN, 0.0), (NAN, NAN), (NAN, NAN), (NAN, NAN)],
];

const COSH_TABLE: SpecialTable = [
    [(INF, NAN), U, (INF, 0.0), (INF, -0.0), U, (INF, NAN), (INF, NAN)],
    [(NAN, NAN), U, U, U, U, (NAN, NAN), (NAN, NAN)],
    [(NAN, 0.0), U, (1.0, 0.0), (1.0, -0.0), U, (NAN, 0.0), (NAN, 0.0)],
    [(NAN, 0.0), U, (1.0, -0.0), (1.0, 0.0), U, (NAN, 0.0), (NAN, 0.0)],
    [(NAN, NAN), U, U, U, U, (NAN, NAN), (NAN, NAN)],
    [(INF, NAN), U, (INF, -0.0), (INF, 0.0), U, (INF, NAN), (INF, NAN)],
    [(NAN, NAN), (NAN, NAN), (NAN, 0.0), (NAN, 0.0), (NAN, NAN), (NAN, NAN), (NAN, NAN)],
];

fn c_sinh(re: f64, im: f64) -> PyResult<(f64, f64)> {
    if !re.is_finite() || !im.is_finite() {
        if re.is_infinite() && im.is_finite() && im != 0.0 {
            if re > 0.0 {
                return Ok((INF.copysign(im.cos()), INF.copysign(im.sin())));
            } else {
                return Ok((-(INF.copysign(im.cos())), INF.copysign(im.sin())));
            }
        }
        if im.is_infinite() && !re.is_nan() {
            return Err(PyError::value_error("math domain error"));
        }
        if let Some(v) = special_lookup(&SINH_TABLE, re, im) {
            return Ok(v);
        }
    }
    let (r, i) = if re.abs() > f64::MAX.ln() {
        let x1 = re - 1.0f64.copysign(re);
        (
            im.cos() * x1.sinh() * std::f64::consts::E,
            im.sin() * x1.cosh() * std::f64::consts::E,
        )
    } else {
        (im.cos() * re.sinh(), im.sin() * re.cosh())
    };
    if r.is_infinite() || i.is_infinite() {
        return Err(PyError::overflow_error("math range error"));
    }
    Ok((r, i))
}

fn c_cosh(re: f64, im: f64) -> PyResult<(f64, f64)> {
    if !re.is_finite() || !im.is_finite() {
        if re.is_infinite() && im.is_finite() && im != 0.0 {
            if re > 0.0 {
                return Ok((INF.copysign(im.cos()), INF.copysign(im.sin())));
            } else {
                return Ok((INF.copysign(im.cos()), -(INF.copysign(im.sin()))));
            }
        }
        if im.is_infinite() && !re.is_nan() {
            return Err(PyError::value_error("math domain error"));
        }
        if let Some(v) = special_lookup(&COSH_TABLE, re, im) {
            return Ok(v);
        }
    }
    let (r, i) = if re.abs() > f64::MAX.ln() {
        let x1 = re - 1.0f64.copysign(re);
        (
            im.cos() * x1.cosh() * std::f64::consts::E,
            im.sin() * x1.sinh() * std::f64::consts::E,
        )
    } else {
        (im.cos() * re.cosh(), im.sin() * re.sinh())
    };
    if r.is_infinite() || i.is_infinite() {
        return Err(PyError::overflow_error("math range error"));
    }
    Ok((r, i))
}

fn c_asinh(re: f64, im: f64) -> (f64, f64) {
    if let Some(v) = special_lookup(&ASINH_TABLE, re, im) {
        return v;
    }
    if re.abs() > CM_LARGE_DOUBLE || im.abs() > CM_LARGE_DOUBLE {
        // Avoid overflow: log(hypot(re/2, im/2)) + 2*ln2, signed per CPython.
        let h = (re / 2.0).hypot(im / 2.0);
        let rr = if im >= 0.0 {
            (h.ln() + std::f64::consts::LN_2 * 2.0).copysign(re)
        } else {
            -((h.ln() + std::f64::consts::LN_2 * 2.0).copysign(-re))
        };
        return (rr, im.atan2(re.abs()));
    }
    let (s1r, s1i) = c_sqrt(1.0 + im, -re);
    let (s2r, s2i) = c_sqrt(1.0 - im, re);
    let r = (s1r * s2i - s2r * s1i).asinh();
    let i = im.atan2(s1r * s2r - s1i * s2i);
    (r, i)
}

const ACOSH_TABLE: SpecialTable = [
    [(INF, -P34), (INF, -P), (INF, -P), (INF, P), (INF, P), (INF, P34), (INF, NAN)],
    [(INF, -P12), U, U, U, U, (INF, P12), (NAN, NAN)],
    [(INF, -P12), U, (0.0, -P12), (0.0, P12), U, (INF, P12), (NAN, P12)],
    [(INF, -P12), U, (0.0, -P12), (0.0, P12), U, (INF, P12), (NAN, P12)],
    [(INF, -P12), U, U, U, U, (INF, P12), (NAN, NAN)],
    [(INF, -P14), (INF, -0.0), (INF, -0.0), (INF, 0.0), (INF, 0.0), (INF, P14), (INF, NAN)],
    [(INF, NAN), (NAN, NAN), (NAN, NAN), (NAN, NAN), (NAN, NAN), (INF, NAN), (NAN, NAN)],
];

fn c_acosh(re: f64, im: f64) -> (f64, f64) {
    if let Some(v) = special_lookup(&ACOSH_TABLE, re, im) {
        return v;
    }
    if re.abs() > CM_LARGE_DOUBLE || im.abs() > CM_LARGE_DOUBLE {
        let h = (re / 2.0).hypot(im / 2.0);
        let r = h.ln() + std::f64::consts::LN_2 * 2.0;
        let i = im.atan2(re);
        return (r, i);
    }
    let (s1r, s1i) = c_sqrt(re - 1.0, im);
    let (s2r, s2i) = c_sqrt(re + 1.0, im);
    let r = (s1r * s2r + s1i * s2i).asinh();
    let i = 2.0 * s1i.atan2(s2r);
    (r, i)
}

// asin(z) = -i * asinh(i*z)
fn c_asin(re: f64, im: f64) -> (f64, f64) {
    let (sr, si) = c_asinh(-im, re);
    (si, -sr)
}

// acos(z) via two sqrt calls (CPython cmath_acos_impl).
fn c_acos(re: f64, im: f64) -> (f64, f64) {
    if let Some(v) = special_lookup(&ACOS_TABLE, re, im) {
        return v;
    }
    if re.abs() > CM_LARGE_DOUBLE || im.abs() > CM_LARGE_DOUBLE {
        let r = im.abs().atan2(re);
        let h = (re / 2.0).hypot(im / 2.0);
        let i = -((h.ln() + std::f64::consts::LN_2 * 2.0).copysign(im));
        return (r, i);
    }
    let (s1r, s1i) = c_sqrt(1.0 - re, -im);
    let (s2r, s2i) = c_sqrt(1.0 + re, im);
    let r = 2.0 * s1r.atan2(s2r);
    let i = (s2r * s1i - s2i * s1r).asinh();
    (r, i)
}

fn c_atanh(re: f64, im: f64) -> PyResult<(f64, f64)> {
    if let Some(v) = special_lookup(&ATANH_TABLE, re, im) {
        return Ok(v);
    }
    // Reduce to re >= 0 via atanh(z) = -atanh(-z).
    if re < 0.0 {
        let (nr, ni) = c_atanh(-re, -im)?;
        return Ok((-nr, -ni));
    }
    let ay = im.abs();
    if re > cm_sqrt_large_double() || ay > cm_sqrt_large_double() {
        let h = (re / 2.0).hypot(im / 2.0);
        let r = re / 4.0 / h / h;
        let i = (P / 2.0).copysign(im);
        return Ok((r, i));
    }
    if re == 1.0 && ay < CM_SQRT_DBL_MIN {
        if ay == 0.0 {
            // CPython sets errno=EDOM here; the clinic wrapper turns it
            // into ValueError("math domain error").
            return Err(PyError::value_error("math domain error"));
        }
        let r = -((ay.sqrt() / ay.hypot(2.0).sqrt()).ln());
        let i = ((2.0f64).atan2(-ay) / 2.0).copysign(im);
        return Ok((r, i));
    }
    let r = (4.0 * re / ((1.0 - re) * (1.0 - re) + ay * ay)).ln_1p() / 4.0;
    let i = -(-2.0 * im).atan2((1.0 - re) * (1.0 + re) - ay * ay) / 2.0;
    Ok((r, i))
}

// atan(z) = -i * atanh(i*z)
fn c_atan(re: f64, im: f64) -> PyResult<(f64, f64)> {
    let (sr, si) = c_atanh(-im, re)?;
    Ok((si, -sr))
}

pub fn create_cmath_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! cm_func {
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

    d.insert("pi".to_string(), py_float(std::f64::consts::PI));
    d.insert("e".to_string(), py_float(std::f64::consts::E));
    d.insert("tau".to_string(), py_float(std::f64::consts::TAU));
    d.insert(
        "inf".to_string(),
        py_float(f64::INFINITY),
    );
    d.insert("nan".to_string(), py_float(f64::NAN));
    d.insert(
        "infj".to_string(),
        py_complex_or_float(0.0, f64::INFINITY),
    );
    d.insert("nanj".to_string(), py_complex_or_float(0.0, f64::NAN));

    cm_func!("sqrt", |args| {
        let (re, im) = arg_complex(args, 0, "sqrt")?;
        let (r, i) = c_sqrt(re, im);
        Ok(py_complex_or_float(r, i))
    });
    cm_func!("sin", |args| {
        // CPython: sin(z) = -i*sinh(i*z).
        let (re, im) = arg_complex(args, 0, "sin")?;
        let (sr, si) = c_sinh(-im, re)?;
        Ok(py_complex_or_float(si, -sr))
    });
    cm_func!("cos", |args| {
        // CPython: cos(z) = cosh(i*z) (routes inf/nan through cosh's
        // table + EDOM handling).
        let (re, im) = arg_complex(args, 0, "cos")?;
        let (r, i) = c_cosh(-im, re)?;
        Ok(py_complex_or_float(r, i))
    });
    cm_func!("tan", |args| {
        // CPython: tan(z) = -i*tanh(i*z).
        let (re, im) = arg_complex(args, 0, "tan")?;
        let (sr, si) = c_tanh(-im, re)?;
        Ok(py_complex_or_float(si, -sr))
    });
    cm_func!("tanh", |args| {
        let (re, im) = arg_complex(args, 0, "tanh")?;
        let (r, i) = c_tanh(re, im)?;
        Ok(py_complex_or_float(r, i))
    });
    cm_func!("sinh", |args| {
        let (re, im) = arg_complex(args, 0, "sinh")?;
        let (r, i) = c_sinh(re, im)?;
        Ok(py_complex_or_float(r, i))
    });
    cm_func!("cosh", |args| {
        let (re, im) = arg_complex(args, 0, "cosh")?;
        let (r, i) = c_cosh(re, im)?;
        Ok(py_complex_or_float(r, i))
    });
    cm_func!("asinh", |args| {
        let (re, im) = arg_complex(args, 0, "asinh")?;
        let (r, i) = c_asinh(re, im);
        Ok(py_complex_or_float(r, i))
    });
    cm_func!("acosh", |args| {
        let (re, im) = arg_complex(args, 0, "acosh")?;
        let (r, i) = c_acosh(re, im);
        Ok(py_complex_or_float(r, i))
    });
    cm_func!("asin", |args| {
        let (re, im) = arg_complex(args, 0, "asin")?;
        let (r, i) = c_asin(re, im);
        Ok(py_complex_or_float(r, i))
    });
    cm_func!("acos", |args| {
        let (re, im) = arg_complex(args, 0, "acos")?;
        let (r, i) = c_acos(re, im);
        Ok(py_complex_or_float(r, i))
    });
    cm_func!("atan", |args| {
        let (re, im) = arg_complex(args, 0, "atan")?;
        let (r, i) = c_atan(re, im)?;
        Ok(py_complex_or_float(r, i))
    });
    cm_func!("atanh", |args| {
        let (re, im) = arg_complex(args, 0, "atanh")?;
        let (r, i) = c_atanh(re, im)?;
        Ok(py_complex_or_float(r, i))
    });
    cm_func!("exp", |args| {
        let (re, im) = arg_complex(args, 0, "exp")?;
        let (r, i) = c_exp(re, im)?;
        Ok(py_complex_or_float(r, i))
    });
    cm_func!("log", |args| {
        let (re, im) = arg_complex(args, 0, "log")?;
        // CPython raises ValueError for log(0) (EDOM); infinities/nans
        // flow through the formula.
        if re == 0.0 && im == 0.0 {
            return Err(PyError::value_error("math domain error"));
        }
        let (r, i) = c_ln(re, im);
        if args.len() > 1 {
            let (bre, bim) = arg_complex(args, 1, "log")?;
            let (lbr, lbi) = c_ln(bre, bim);
            let (qr, qi) = c_div((r, i), (lbr, lbi));
            return Ok(py_complex_or_float(qr, qi));
        }
        Ok(py_complex_or_float(r, i))
    });
    cm_func!("log10", |args| {
        let (re, im) = arg_complex(args, 0, "log10")?;
        if re == 0.0 && im == 0.0 {
            return Err(PyError::value_error("math domain error"));
        }
        let (r, i) = c_ln(re, im);
        let ln10 = std::f64::consts::LN_10;
        Ok(py_complex_or_float(r / ln10, i / ln10))
    });
    cm_func!("phase", |args| {
        let (re, im) = arg_complex(args, 0, "phase")?;
        Ok(py_float(c_phase(re, im)))
    });
    cm_func!("polar", |args| {
        let (re, im) = arg_complex(args, 0, "polar")?;
        // OverflowError on overflow like abs() (test_cmath polar0100).
        let r = c_abs(re, im);
        if r.is_infinite() && re.is_finite() && im.is_finite() {
            return Err(PyError::overflow_error("absolute value too large"));
        }
        Ok(PyObjectRef::imm(PyObject::Tuple(vec![
            py_float(r),
            py_float(c_phase(re, im)),
        ])))
    });
    cm_func!("rect", |args| {
        if args.len() != 2 {
            return Err(PyError::type_error("rect() takes exactly 2 arguments"));
        }
        let r = args[0]
            .as_f64()
            .ok_or_else(|| PyError::type_error("rect() argument must be a number"))?;
        let phi = args[1]
            .as_f64()
            .ok_or_else(|| PyError::type_error("rect() argument must be a number"))?;
        // CPython cmath_rect_impl (special values, EDOM, -0 workaround).
        if !r.is_finite() || !phi.is_finite() {
            if r.is_infinite() && phi.is_finite() && phi != 0.0 {
                let (rr, ii) = if r > 0.0 {
                    (INF.copysign(phi.cos()), INF.copysign(phi.sin()))
                } else {
                    (-(INF.copysign(phi.cos())), -(INF.copysign(phi.sin())))
                };
                return Ok(py_complex_or_float(rr, ii));
            }
            if phi.is_infinite() && r != 0.0 && !r.is_nan() {
                return Err(PyError::value_error("math domain error"));
            }
            const RECT_TABLE: SpecialTable = [
                [(INF, NAN), U, (-INF, 0.0), (-INF, -0.0), U, (INF, NAN), (INF, NAN)],
                [(NAN, NAN), U, U, U, U, (NAN, NAN), (NAN, NAN)],
                [(0.0, 0.0), U, (-0.0, 0.0), (-0.0, -0.0), U, (0.0, 0.0), (0.0, 0.0)],
                [(0.0, 0.0), U, (0.0, -0.0), (0.0, 0.0), U, (0.0, 0.0), (0.0, 0.0)],
                [(NAN, NAN), U, U, U, U, (NAN, NAN), (NAN, NAN)],
                [(INF, NAN), U, (INF, -0.0), (INF, 0.0), U, (INF, NAN), (INF, NAN)],
                [(NAN, NAN), (NAN, NAN), (NAN, 0.0), (NAN, 0.0), (NAN, NAN), (NAN, NAN), (NAN, NAN)],
            ];
            if let Some((rr, ii)) = special_lookup(&RECT_TABLE, r, phi) {
                return Ok(py_complex_or_float(rr, ii));
            }
        } else if phi == 0.0 {
            // Preserve -0.0 sign (OSX workaround in CPython).
            return Ok(py_complex_or_float(r, r * phi));
        }
        Ok(py_complex_or_float(r * phi.cos(), r * phi.sin()))
    });
    cm_func!("isnan", |args| {
        let (re, im) = arg_complex(args, 0, "isnan")?;
        Ok(py_bool(re.is_nan() || im.is_nan()))
    });
    cm_func!("isinf", |args| {
        let (re, im) = arg_complex(args, 0, "isinf")?;
        Ok(py_bool(re.is_infinite() || im.is_infinite()))
    });
    cm_func!("isfinite", |args| {
        let (re, im) = arg_complex(args, 0, "isfinite")?;
        Ok(py_bool(re.is_finite() && im.is_finite()))
    });
    cm_func!("isclose", |args| {
        let (are, aim) = arg_complex(args, 0, "isclose")?;
        let (bre, bim) = arg_complex(args, 1, "isclose")?;
        // Keyword args arrive packed into a trailing dict per this
        // codebase's `BuiltinFunction` calling convention (see
        // `math.isclose` in `src/modules/core/math.rs` for the same
        // pattern).
        let mut rel_tol = 1e-9;
        let mut abs_tol = 0.0;
        if let Some(last) = args.last() {
            if let PyObject::Dict(kwargs) = &*last.borrow() {
                if let Ok(Some(v)) = kwargs.get(&py_str("rel_tol")) {
                    let b = v.borrow();
                    let (r, i) = as_complex_parts(&b)
                        .ok_or_else(|| PyError::type_error("isclose() argument must be a number"))?;
                    if i != 0.0 {
                        return Err(PyError::type_error(
                            "isclose() argument must be a number",
                        ));
                    }
                    rel_tol = r;
                }
                if let Ok(Some(v)) = kwargs.get(&py_str("abs_tol")) {
                    let b = v.borrow();
                    let (r, i) = as_complex_parts(&b)
                        .ok_or_else(|| PyError::type_error("isclose() argument must be a number"))?;
                    if i != 0.0 {
                        return Err(PyError::type_error(
                            "isclose() argument must be a number",
                        ));
                    }
                    abs_tol = r;
                }
            }
        }
        if rel_tol < 0.0 || abs_tol < 0.0 {
            return Err(PyError::value_error("tolerances must be non-negative"));
        }
        if are == bre && aim == bim {
            return Ok(py_bool(true));
        }
        if are.is_infinite() || aim.is_infinite() || bre.is_infinite() || bim.is_infinite() {
            return Ok(py_bool(false));
        }
        let diff = c_abs(are - bre, aim - bim);
        let a_abs = c_abs(are, aim);
        let b_abs = c_abs(bre, bim);
        Ok(py_bool(
            diff <= (rel_tol * a_abs.max(b_abs)).max(abs_tol),
        ))
    });

    d
}
