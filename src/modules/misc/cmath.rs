use crate::object::*;
use std::collections::HashMap;

fn arg_complex(args: &[PyObjectRef], idx: usize, fname: &str) -> Result<(f64, f64), PyError> {
    let v = args
        .get(idx)
        .ok_or_else(|| PyError::type_error(format!("{fname}() missing argument")))?;
    let b = v.borrow();
    as_complex_parts(&b)
        .ok_or_else(|| PyError::type_error(format!("{fname}() argument must be a number")))
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
    (c_abs(re, im).ln(), c_phase(re, im))
}

fn c_exp(re: f64, im: f64) -> (f64, f64) {
    let mag = re.exp();
    (mag * im.cos(), mag * im.sin())
}

fn c_mul(a: (f64, f64), b: (f64, f64)) -> (f64, f64) {
    (a.0 * b.0 - a.1 * b.1, a.0 * b.1 + a.1 * b.0)
}

fn c_div(a: (f64, f64), b: (f64, f64)) -> (f64, f64) {
    let denom = b.0 * b.0 + b.1 * b.1;
    (
        (a.0 * b.0 + a.1 * b.1) / denom,
        (a.1 * b.0 - a.0 * b.1) / denom,
    )
}

fn c_add(a: (f64, f64), b: (f64, f64)) -> (f64, f64) {
    (a.0 + b.0, a.1 + b.1)
}

fn c_sub(a: (f64, f64), b: (f64, f64)) -> (f64, f64) {
    (a.0 - b.0, a.1 - b.1)
}

// tanh(x+iy) = (sinh(2x) + i*sin(2y)) / (cosh(2x) + cos(2y))
fn c_tanh(re: f64, im: f64) -> (f64, f64) {
    let denom = (2.0 * re).cosh() + (2.0 * im).cos();
    ((2.0 * re).sinh() / denom, (2.0 * im).sin() / denom)
}

// atanh(z) = 0.5 * ln((1+z)/(1-z))
fn c_atanh(re: f64, im: f64) -> (f64, f64) {
    let num = c_add((1.0, 0.0), (re, im));
    let den = c_sub((1.0, 0.0), (re, im));
    let (qr, qi) = c_div(num, den);
    let (lr, li) = c_ln(qr, qi);
    (0.5 * lr, 0.5 * li)
}

// atan(z) = -i * atanh(i*z)
fn c_atan(re: f64, im: f64) -> (f64, f64) {
    let (ir, ii) = (-im, re); // i*z
    let (ar, ai) = c_atanh(ir, ii);
    (ai, -ar) // -i * (ar+ai*i)
}

fn c_sqrt(re: f64, im: f64) -> (f64, f64) {
    if im == 0.0 && re >= 0.0 {
        return (re.sqrt(), 0.0);
    }
    let r = c_abs(re, im);
    let sr = ((r + re) / 2.0).sqrt();
    let si = ((r - re) / 2.0).sqrt();
    (sr, if im < 0.0 { -si } else { si })
}

fn c_sinh(re: f64, im: f64) -> (f64, f64) {
    (re.sinh() * im.cos(), re.cosh() * im.sin())
}

fn c_cosh(re: f64, im: f64) -> (f64, f64) {
    (re.cosh() * im.cos(), re.sinh() * im.sin())
}

// asinh(z) = ln(z + sqrt(z^2+1))
fn c_asinh(re: f64, im: f64) -> (f64, f64) {
    let z2 = c_mul((re, im), (re, im));
    let inner = c_add(z2, (1.0, 0.0));
    let (sr, si) = c_sqrt(inner.0, inner.1);
    c_ln(re + sr, im + si)
}

// acosh(z) = ln(z + sqrt(z^2-1))
fn c_acosh(re: f64, im: f64) -> (f64, f64) {
    let z2 = c_mul((re, im), (re, im));
    let inner = c_sub(z2, (1.0, 0.0));
    let (sr, si) = c_sqrt(inner.0, inner.1);
    c_ln(re + sr, im + si)
}

// asin(z) = -i * ln(iz + sqrt(1 - z^2))
fn c_asin(re: f64, im: f64) -> (f64, f64) {
    let z2 = c_mul((re, im), (re, im));
    let inner = c_sub((1.0, 0.0), z2);
    let (sr, si) = c_sqrt(inner.0, inner.1);
    let (lr, li) = c_ln(-im + sr, re + si); // iz + sqrt(...)
    (li, -lr) // -i * (lr + li*i)
}

// acos(z) = pi/2 - asin(z)
fn c_acos(re: f64, im: f64) -> (f64, f64) {
    let (sr, si) = c_asin(re, im);
    (std::f64::consts::FRAC_PI_2 - sr, -si)
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
        let (re, im) = arg_complex(args, 0, "sin")?;
        Ok(py_complex_or_float(
            re.sin() * im.cosh(),
            re.cos() * im.sinh(),
        ))
    });
    cm_func!("cos", |args| {
        let (re, im) = arg_complex(args, 0, "cos")?;
        Ok(py_complex_or_float(
            re.cos() * im.cosh(),
            -re.sin() * im.sinh(),
        ))
    });
    cm_func!("tan", |args| {
        let (re, im) = arg_complex(args, 0, "tan")?;
        let (sr, si) = (re.sin() * im.cosh(), re.cos() * im.sinh());
        let (cr, ci) = (re.cos() * im.cosh(), -re.sin() * im.sinh());
        let (r, i) = c_div((sr, si), (cr, ci));
        Ok(py_complex_or_float(r, i))
    });
    cm_func!("tanh", |args| {
        let (re, im) = arg_complex(args, 0, "tanh")?;
        let (r, i) = c_tanh(re, im);
        Ok(py_complex_or_float(r, i))
    });
    cm_func!("sinh", |args| {
        let (re, im) = arg_complex(args, 0, "sinh")?;
        let (r, i) = c_sinh(re, im);
        Ok(py_complex_or_float(r, i))
    });
    cm_func!("cosh", |args| {
        let (re, im) = arg_complex(args, 0, "cosh")?;
        let (r, i) = c_cosh(re, im);
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
        let (r, i) = c_atan(re, im);
        Ok(py_complex_or_float(r, i))
    });
    cm_func!("atanh", |args| {
        let (re, im) = arg_complex(args, 0, "atanh")?;
        let (r, i) = c_atanh(re, im);
        Ok(py_complex_or_float(r, i))
    });
    cm_func!("exp", |args| {
        let (re, im) = arg_complex(args, 0, "exp")?;
        let (r, i) = c_exp(re, im);
        Ok(py_complex_or_float(r, i))
    });
    cm_func!("log", |args| {
        let (re, im) = arg_complex(args, 0, "log")?;
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
        Ok(PyObjectRef::imm(PyObject::Tuple(vec![
            py_float(c_abs(re, im)),
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
