use crate::object::*;
use std::collections::HashMap;
use std::sync::atomic::AtomicI64;
use num_traits::{Signed, ToPrimitive};
use std::rc::Rc;


mod helpers;
mod special;
use helpers::{dl_fast_sum, exact_fsum, log_frexp_int, math_arg_f64, math_call_int_dunder, math_float_value, math_int_value, math_log2_value, vector_norm};


pub fn create_math_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! math_func {
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
    math_func!("sqrt", |args| {
        if args.len() != 1 {
            return Err(PyError::type_error("sqrt() takes exactly one argument"));
        }
        let v = args[0].borrow();
        match &*v {
            PyObject::Int(i) => {
                let f = i.to_f64().unwrap_or(0.0);
                if f < 0.0 {
                    return Err(PyError::value_error(format!(
                        "expected a nonnegative input, got {:?}",
                        f
                    )));
                }
                Ok(py_float(f.sqrt()))
            }
            PyObject::Float(f) => {
                if *f < 0.0 {
                    return Err(PyError::value_error(format!(
                        "expected a nonnegative input, got {:?}",
                        f
                    )));
                }
                Ok(py_float(f.sqrt()))
            }
            _ => Err(PyError::type_error("sqrt() argument must be a number")),
        }
    });
    math_func!("sin", |args| {
        if args.len() != 1 {
            return Err(PyError::type_error("sin() takes exactly one argument"));
        }
        let v = args[0].borrow();
        match &*v {
            PyObject::Int(i) => Ok(py_float(i.to_f64().unwrap_or(0.0).sin())),
            PyObject::Float(f) => {
                if f.is_infinite() {
                    return Err(PyError::value_error("math domain error"));
                }
                Ok(py_float(f.sin()))
            }
            _ => Err(PyError::type_error("sin() argument must be a number")),
        }
    });
    math_func!("cos", |args| {
        if args.len() != 1 {
            return Err(PyError::type_error("cos() takes exactly one argument"));
        }
        let v = args[0].borrow();
        match &*v {
            PyObject::Int(i) => Ok(py_float(i.to_f64().unwrap_or(0.0).cos())),
            PyObject::Float(f) => {
                if f.is_infinite() {
                    return Err(PyError::value_error("math domain error"));
                }
                Ok(py_float(f.cos()))
            }
            _ => Err(PyError::type_error("cos() argument must be a number")),
        }
    });
    math_func!("tan", |args| {
        if args.len() != 1 {
            return Err(PyError::type_error("tan() takes exactly one argument"));
        }
        let v = args[0].borrow();
        match &*v {
            PyObject::Int(i) => Ok(py_float(i.to_f64().unwrap_or(0.0).tan())),
            PyObject::Float(f) => {
                if f.is_infinite() {
                    return Err(PyError::value_error("math domain error"));
                }
                Ok(py_float(f.tan()))
            }
            _ => Err(PyError::type_error("tan() argument must be a number")),
        }
    });
    math_func!("floor", |args| {
        if args.len() != 1 {
            return Err(PyError::type_error("floor() takes exactly one argument"));
        }
        if let Some(r) = math_call_int_dunder(&args[0], "__floor__")? {
            return Ok(r);
        }
        let v = args[0].borrow();
        match &*v {
            PyObject::Int(i) => Ok(py_int(i.clone())),
            PyObject::Float(f) => crate::object::f64_to_int_ceil_floor_trunc(*f, 2).map(py_int),
            _ => {
                let x = math_arg_f64(&args[0])
                    .ok_or_else(|| PyError::type_error("floor() argument must be a number"))?;
                crate::object::f64_to_int_ceil_floor_trunc(x, 2).map(py_int)
            }
        }
    });
    math_func!("ceil", |args| {
        if args.len() != 1 {
            return Err(PyError::type_error("ceil() takes exactly one argument"));
        }
        if let Some(r) = math_call_int_dunder(&args[0], "__ceil__")? {
            return Ok(r);
        }
        let v = args[0].borrow();
        match &*v {
            PyObject::Int(i) => Ok(py_int(i.clone())),
            PyObject::Float(f) => crate::object::f64_to_int_ceil_floor_trunc(*f, 1).map(py_int),
            _ => {
                let x = math_arg_f64(&args[0])
                    .ok_or_else(|| PyError::type_error("ceil() argument must be a number"))?;
                crate::object::f64_to_int_ceil_floor_trunc(x, 1).map(py_int)
            }
        }
    });
    math_func!("exp", |args| {
        if args.len() != 1 {
            return Err(PyError::type_error("exp() takes exactly one argument"));
        }
        let v = args[0].borrow();
        let result = match &*v {
            PyObject::Int(i) => i.to_f64().unwrap_or(0.0).exp(),
            PyObject::Float(f) => f.exp(),
            _ => return Err(PyError::type_error("exp() argument must be a number")),
        };
        let x = math_arg_f64(&args[0]).unwrap_or(f64::NAN);
        if result.is_infinite() && x.is_finite() {
            return Err(PyError::overflow_error("math range error"));
        }
        Ok(py_float(result))
    });
    macro_rules! math_func1 {
        ($name:expr, $f:expr) => {
            math_func!($name, |args| {
                if args.len() != 1 {
                    return Err(PyError::type_error(concat!(
                        $name,
                        "() takes exactly one argument"
                    )));
                }
                let x = math_arg_f64(&args[0]).ok_or_else(|| {
                    PyError::type_error(concat!($name, "() argument must be a number"))
                })?;
                Ok(py_float(($f)(x)))
            });
        };
    }
    math_func1!("cbrt", f64::cbrt);
    math_func!("exp2", |args| {
        if args.len() != 1 {
            return Err(PyError::type_error("exp2() takes exactly one argument"));
        }
        let x = math_arg_f64(&args[0])
            .ok_or_else(|| PyError::type_error("exp2() argument must be a number"))?;
        let result = x.exp2();
        if result.is_infinite() && x.is_finite() {
            return Err(PyError::overflow_error("math range error"));
        }
        Ok(py_float(result))
    });
    math_func1!("erf", libm::erf);
    math_func1!("erfc", libm::erfc);
    math_func!("gamma", |args| {
        if args.len() != 1 {
            return Err(PyError::type_error("gamma() takes exactly one argument"));
        }
        let x = math_arg_f64(&args[0])
            .ok_or_else(|| PyError::type_error("gamma() argument must be a number"))?;
        // gamma of a non-positive integer is a pole (ValueError in
        // CPython), with the double repr in the message; overflow of the
        // result for finite inputs is an OverflowError.
        if x.is_finite() && x <= 0.0 && x == x.trunc() {
            return Err(PyError::value_error(format!(
                "expected a noninteger or positive integer, got {:?}",
                x
            )));
        }
        if x == f64::NEG_INFINITY {
            return Err(PyError::value_error("math domain error"));
        }
        let r = libm::tgamma(x);
        if r.is_infinite() && x.is_finite() {
            return Err(PyError::overflow_error("math range error"));
        }
        Ok(py_float(r))
    });
    math_func!("expm1", |args| {
        if args.len() != 1 {
            return Err(PyError::type_error("expm1() takes exactly one argument"));
        }
        let x = math_arg_f64(&args[0])
            .ok_or_else(|| PyError::type_error("expm1() argument must be a number"))?;
        let r = x.exp_m1();
        if r.is_infinite() && x.is_finite() {
            return Err(PyError::overflow_error("math range error"));
        }
        Ok(py_float(r))
    });
    math_func!("asin", |args| {
        if args.len() != 1 {
            return Err(PyError::type_error("asin() takes exactly one argument"));
        }
        let x = math_arg_f64(&args[0])
            .ok_or_else(|| PyError::type_error("asin() argument must be a number"))?;
        if x < -1.0 || x > 1.0 {
            return Err(PyError::value_error("math domain error"));
        }
        Ok(py_float(x.asin()))
    });
    math_func1!("atan", f64::atan);
    math_func!("sinh", |args| {
        if args.len() != 1 {
            return Err(PyError::type_error("sinh() takes exactly one argument"));
        }
        let x = math_arg_f64(&args[0])
            .ok_or_else(|| PyError::type_error("sinh() argument must be a number"))?;
        let r = x.sinh();
        if r.is_infinite() && x.is_finite() {
            return Err(PyError::overflow_error("math range error"));
        }
        Ok(py_float(r))
    });
    math_func!("cosh", |args| {
        if args.len() != 1 {
            return Err(PyError::type_error("cosh() takes exactly one argument"));
        }
        let x = math_arg_f64(&args[0])
            .ok_or_else(|| PyError::type_error("cosh() argument must be a number"))?;
        let r = x.cosh();
        if r.is_infinite() && x.is_finite() {
            return Err(PyError::overflow_error("math range error"));
        }
        Ok(py_float(r))
    });
    math_func1!("tanh", f64::tanh);
    math_func1!("asinh", f64::asinh);
    math_func!("acosh", |args| {
        if args.len() != 1 {
            return Err(PyError::type_error("acosh() takes exactly one argument"));
        }
        let x = math_arg_f64(&args[0])
            .ok_or_else(|| PyError::type_error("acosh() argument must be a number"))?;
        if x < 1.0 {
            return Err(PyError::value_error("math domain error"));
        }
        Ok(py_float(x.acosh()))
    });
    math_func!("atanh", |args| {
        if args.len() != 1 {
            return Err(PyError::type_error("atanh() takes exactly one argument"));
        }
        let x = math_arg_f64(&args[0])
            .ok_or_else(|| PyError::type_error("atanh() argument must be a number"))?;
        if x <= -1.0 || x >= 1.0 {
            return Err(PyError::value_error(format!(
                "expected a number between -1 and 1, got {:?}",
                x
            )));
        }
        // atanh(x) = 0.5 * (log1p(x) - log1p(-x)): numerically stable and
        // accurate near ±1, unlike the raw libm/`f64::atanh` which loses
        // precision there (test_testfile's atanh0022/0023).
        Ok(py_float(0.5 * (x.ln_1p() - (-x).ln_1p())))
    });
    math_func1!("degrees", f64::to_degrees);
    math_func1!("radians", f64::to_radians);
    math_func!("pow", |args| {
        if args.len() != 2 {
            return Err(PyError::type_error("pow() takes exactly two arguments"));
        }
        let a = args[0].borrow();
        let b = args[1].borrow();
        let (x, y) = match (&*a, &*b) {
            (PyObject::Int(i), PyObject::Int(j)) => {
                (i.to_f64().unwrap_or(0.0), j.to_f64().unwrap_or(0.0))
            }
            (PyObject::Int(i), PyObject::Float(f)) => (i.to_f64().unwrap_or(0.0), *f),
            (PyObject::Float(f), PyObject::Int(i)) => (*f, i.to_f64().unwrap_or(0.0)),
            (PyObject::Float(a), PyObject::Float(b)) => (*a, *b),
            _ => return Err(PyError::type_error("pow() argument must be a number")),
        };
        // `0 ** negative` is a real domain error (division by zero), not a
        // silent `inf`/`nan` — matches real CPython's own `math.pow`
        // (`ValueError: math domain error`).
        // Only a FINITE negative exponent is a domain error — `0.0 **
        // -inf` legitimately diverges to `inf` (matches the underlying
        // C `pow()` and real CPython's own `math.pow(0., NINF) == INF`).
        if x == 0.0 && y < 0.0 && y.is_finite() {
            return Err(PyError::value_error("math domain error"));
        }
        // A negative base raised to a finite, non-integer exponent has no
        // real result (it's genuinely complex) — real CPython's `math.pow`
        // raises `ValueError: math domain error` here too, rather than the
        // `NaN` plain `f64::powf` produces.
        if x < 0.0 && x.is_finite() && y.is_finite() && y.fract() != 0.0 {
            return Err(PyError::value_error("math domain error"));
        }
        let result = x.powf(y);
        // A genuine overflow (both inputs finite, result isn't) must raise
        // `OverflowError`, not silently return `inf` — legitimate infinite
        // results (`pow(INF, 1)`, `pow(x, INF)`, etc.) are unaffected since
        // at least one input is already infinite in those cases. Found via
        // CPython's own `test_math.py::testPow` (`math.pow(1e+100,
        // 1e+100)`).
        if result.is_infinite() && x.is_finite() && y.is_finite() {
            return Err(PyError::overflow_error("math range error"));
        }
        Ok(py_float(result))
    });
    math_func!("fma", |args| {
        if args.len() != 3 {
            return Err(PyError::type_error("fma() takes exactly three arguments"));
        }
        let a = math_arg_f64(&args[0])
            .ok_or_else(|| PyError::type_error("fma() arguments must be numbers"))?;
        let b = math_arg_f64(&args[1])
            .ok_or_else(|| PyError::type_error("fma() arguments must be numbers"))?;
        let c = math_arg_f64(&args[2])
            .ok_or_else(|| PyError::type_error("fma() arguments must be numbers"))?;
        // A NaN input takes precedence over every domain error:
        // fma(inf, 0.0, nan) is NaN, but fma(inf, 0.0, 5.0) is ValueError.
        if a.is_nan() || b.is_nan() || c.is_nan() {
            return Ok(py_float(f64::NAN));
        }
        // IEEE-754 fma domain errors (CPython raises ValueError):
        //  - inf * 0 in either order;
        //  - a*b = ±inf with c the opposite-sign infinity (inf + -inf).
        if a.is_infinite() && b == 0.0 || b.is_infinite() && a == 0.0 {
            return Err(PyError::value_error("math domain error"));
        }
        let ab_is_inf = (a.is_infinite() && b != 0.0) || (b.is_infinite() && a != 0.0);
        let ab_sign = if a.is_sign_positive() == b.is_sign_positive() {
            f64::INFINITY
        } else {
            f64::NEG_INFINITY
        };
        if ab_is_inf && c.is_infinite() && ab_sign != c {
            return Err(PyError::value_error("math domain error"));
        }
        // Rust's `mul_add` computes the single-rounded, exact fma (the
        // intermediate product never overflows), so a*b + c exactly is the
        // result; raise OverflowError only when the FINAL result overflows
        // with all inputs finite.
        let result = a.mul_add(b, c);
        if result.is_infinite() && a.is_finite() && b.is_finite() && c.is_finite() {
            return Err(PyError::overflow_error("math range error"));
        }
        Ok(py_float(result))
    });
    math_func!("log", |args| {
        if args.len() < 1 || args.len() > 2 {
            return Err(PyError::type_error("log() takes one or two arguments"));
        }
        let ln_x = {
            let b = args[0].borrow();
            if let PyObject::Int(i) = &*b {
                if i.sign() == num_bigint::Sign::NoSign || i.sign() == num_bigint::Sign::Minus {
                    return Err(PyError::value_error("expected a positive input"));
                }
                let (m, e) = log_frexp_int(i);
                m.ln() + e * std::f64::consts::LN_2
            } else {
                let x = math_arg_f64(&args[0])
                    .ok_or_else(|| PyError::type_error("log() argument must be a number"))?;
                if x <= 0.0 {
                    return Err(PyError::value_error(format!(
                        "expected a positive input, got {:?}",
                        x
                    )));
                }
                x.ln()
            }
        };
        if args.len() == 2 {
            let base = math_arg_f64(&args[1])
                .ok_or_else(|| PyError::type_error("log() base must be a number"))?;
            if base <= 0.0 || base == 1.0 {
                return Err(PyError::value_error(format!(
                    "expected a positive input, got {:?}",
                    base
                )));
            }
            return Ok(py_float(ln_x / base.ln()));
        }
        Ok(py_float(ln_x))
    });
    math_func!("log2", |args| {
        if args.len() != 1 {
            return Err(PyError::type_error("log2() takes exactly one argument"));
        }
        Ok(py_float(math_log2_value(&args[0])?))
    });
    math_func!("log10", |args| {
        if args.len() != 1 {
            return Err(PyError::type_error("log10() takes exactly one argument"));
        }
        let b = args[0].borrow();
        if let PyObject::Int(i) = &*b {
            if i.sign() == num_bigint::Sign::NoSign || i.sign() == num_bigint::Sign::Minus {
                return Err(PyError::value_error("expected a positive input"));
            }
            let (m, e) = log_frexp_int(i);
            return Ok(py_float(m.log10() + e * std::f64::consts::LOG10_2));
        }
        let x = math_arg_f64(&args[0])
            .ok_or_else(|| PyError::type_error("log10() argument must be a number"))?;
        if x <= 0.0 {
            return Err(PyError::value_error(format!(
                "expected a positive input, got {:?}",
                x
            )));
        }
        Ok(py_float(x.log10()))
    });
    math_func!("log1p", |args| {
        if args.len() != 1 {
            return Err(PyError::type_error("log1p() takes exactly one argument"));
        }
        let x = math_arg_f64(&args[0])
            .ok_or_else(|| PyError::type_error("log1p() argument must be a number"))?;
        if x <= -1.0 {
            return Err(PyError::value_error(format!(
                "expected argument value > -1, got {:?}",
                x
            )));
        }
        Ok(py_float(x.ln_1p()))
    });
    math_func!("abs", |args| {
        if args.len() != 1 {
            return Err(PyError::type_error("abs() takes exactly one argument"));
        }
        let v = args[0].borrow();
        match &*v {
            PyObject::Int(i) => Ok(py_float(i.to_f64().unwrap_or(0.0).abs())),
            PyObject::Float(f) => Ok(py_float(f.abs())),
            _ => Err(PyError::type_error("abs() argument must be a number")),
        }
    });
    math_func!("acos", |args| {
        if args.len() != 1 {
            return Err(PyError::type_error("acos() takes exactly one argument"));
        }
        let x = math_arg_f64(&args[0])
            .ok_or_else(|| PyError::type_error("acos() argument must be a number"))?;
        if x < -1.0 || x > 1.0 {
            return Err(PyError::value_error("math domain error"));
        }
        Ok(py_float(x.acos()))
    });
    math_func!("fabs", |args| {
        if args.len() != 1 {
            return Err(PyError::type_error("fabs() takes exactly one argument"));
        }
        let v = args[0].borrow();
        match &*v {
            PyObject::Int(i) => Ok(py_float(i.to_f64().unwrap_or(0.0).abs())),
            PyObject::Float(f) => Ok(py_float(f.abs())),
            _ => Err(PyError::type_error("fabs() argument must be a number")),
        }
    });
    math_func!("isfinite", |args| {
        if args.len() != 1 {
            return Err(PyError::type_error("isfinite() takes exactly one argument"));
        }
        // Ints (including SmallInt, big Int, bool) are always finite.
        // Check type directly – `as_i64` alone would also be Some for a float
        // subclass that happens to be e.g. 1.0, so guard with is_float_typed.
        let is_int_like = {
            let b = args[0].borrow();
            matches!(&*b, PyObject::Int(_) | PyObject::Bool(_))
                || matches!(args[0], PyObjectRef::SmallInt(_)| PyObjectRef::SmallBool(_))
        };
        if is_int_like && !args[0].is_float_typed() {
            return Ok(py_bool(true));
        }
        // Fast path for plain floats (SmallFloat / Float)
        if args[0].is_float_typed() || matches!(&*args[0].borrow(), PyObject::Float(_)) || matches!(args[0], PyObjectRef::SmallFloat(_)) {
            if let Some(f) = args[0].as_f64() {
                return Ok(py_bool(f.is_finite()));
            }
        }
        // Generic path: handles float subclasses (MyFloat), Decimal via
        // __float__, Fractions, etc. `math_arg_f64` dispatches through
        // __float__ for Instances and handles int->float conversion without
        // needing type-specific branches.
        if let Some(f) = math_arg_f64(&args[0]) {
            return Ok(py_bool(f.is_finite()));
        }
        Err(PyError::type_error("isfinite() argument must be a number"))
    });
    math_func!("lgamma", |args| {
        if args.len() != 1 {
            return Err(PyError::type_error("lgamma() takes exactly one argument"));
        }
        let v = args[0].borrow();
        let x = match &*v {
            PyObject::Int(i) => i.to_f64().unwrap_or(0.0),
            PyObject::Float(f) => *f,
            _ => return Err(PyError::type_error("lgamma() argument must be a number")),
        };
        // lgamma is a pole at non-positive integers (CPython raises
        // ValueError there); overflow of the result for finite inputs is
        // an OverflowError.
        if x.is_finite() && x <= 0.0 && x == x.trunc() {
            return Err(PyError::value_error(format!(
                "expected a noninteger or positive integer, got {:?}",
                x
            )));
        }
        let r = libm::lgamma(x);
        if r.is_infinite() && x.is_finite() {
            return Err(PyError::overflow_error("math range error"));
        }
        Ok(py_float(r))
    });
    math_func!("trunc", |args| {
        if args.len() != 1 {
            return Err(PyError::type_error("trunc() takes exactly one argument"));
        }
        let a = &args[0];
        if let Some(r) = math_call_int_dunder(a, "__trunc__")? {
            return Ok(r);
        }
        // A native `float` truncates to the exact integer (like
        // `float.__trunc__`); `int` is its own truncation.
        let v = a.borrow();
        match &*v {
            PyObject::Int(i) => Ok(py_int(i.clone())),
            PyObject::Float(f) => crate::object::f64_to_int_ceil_floor_trunc(*f, 0).map(py_int),
            _ => Err(PyError::type_error(format!(
                "cannot convert '{}' object to int",
                a.borrow().type_name()
            ))),
        }
    });
    math_func!("atan2", |args| {
        if args.len() != 2 {
            return Err(PyError::type_error("atan2() takes exactly two arguments"));
        }
        let y = math_arg_f64(&args[0])
            .ok_or_else(|| PyError::type_error("atan2() argument must be a number"))?;
        let x = math_arg_f64(&args[1])
            .ok_or_else(|| PyError::type_error("atan2() argument must be a number"))?;
        Ok(py_float(y.atan2(x)))
    });
    math_func!("hypot", |args| {
        let mut vals = Vec::with_capacity(args.len());
        for a in args {
            let v = math_arg_f64(&a)
                .ok_or_else(|| PyError::type_error("hypot() arguments must be numbers"))?;
            // An int too big for f64 (10**1000) must raise OverflowError,
            // not silently become +inf.
            if v.is_infinite() && matches!(&*a.borrow(), PyObject::Int(_)) {
                return Err(PyError::overflow_error("int too large to convert to float"));
            }
            vals.push(v);
        }
        // IEEE hypot semantics: any infinity wins, otherwise any NaN wins.
        if vals.iter().any(|v| v.is_infinite()) {
            return Ok(py_float(f64::INFINITY));
        }
        if vals.iter().any(|v| v.is_nan()) {
            return Ok(py_float(f64::NAN));
        }
        if vals.is_empty() {
            return Ok(py_float(0.0));
        }
        let max = vals.iter().cloned().fold(0.0f64, |m, v| m.max(v.abs()));
        if max == 0.0 {
            return Ok(py_float(0.0));
        }
        Ok(py_float(vector_norm(&vals, max)))
    });
    math_func!("copysign", |args| {
        if args.len() != 2 {
            return Err(PyError::type_error(
                "copysign() takes exactly two arguments",
            ));
        }
        let x = math_arg_f64(&args[0])
            .ok_or_else(|| PyError::type_error("copysign() argument must be a number"))?;
        let y = math_arg_f64(&args[1])
            .ok_or_else(|| PyError::type_error("copysign() argument must be a number"))?;
        Ok(py_float(x.copysign(y)))
    });
    math_func!("fmod", |args| {
        if args.len() != 2 {
            return Err(PyError::type_error("fmod() takes exactly two arguments"));
        }
        let x = math_arg_f64(&args[0])
            .ok_or_else(|| PyError::type_error("fmod() argument must be a number"))?;
        let y = math_arg_f64(&args[1])
            .ok_or_else(|| PyError::type_error("fmod() argument must be a number"))?;
        if y == 0.0 || x.is_infinite() {
            return Err(PyError::value_error("math domain error"));
        }
        Ok(py_float(x % y))
    });
    math_func!("isnan", |args| {
        if args.len() != 1 {
            return Err(PyError::type_error("isnan() takes exactly one argument"));
        }
        let x = math_arg_f64(&args[0])
            .ok_or_else(|| PyError::type_error("isnan() argument must be a number"))?;
        Ok(py_bool(x.is_nan()))
    });
    math_func!("isinf", |args| {
        if args.len() != 1 {
            return Err(PyError::type_error("isinf() takes exactly one argument"));
        }
        let x = math_arg_f64(&args[0])
            .ok_or_else(|| PyError::type_error("isinf() argument must be a number"))?;
        Ok(py_bool(x.is_infinite()))
    });
    math_func!("isclose", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error(
                "isclose() takes at least two arguments",
            ));
        }
        let a = math_arg_f64(&args[0])
            .ok_or_else(|| PyError::type_error("isclose() argument must be a number"))?;
        let b = math_arg_f64(&args[1])
            .ok_or_else(|| PyError::type_error("isclose() argument must be a number"))?;
        // `rel_tol`/`abs_tol` (real `math.isclose`'s signature: `isclose(a,
        // b, *, rel_tol=1e-09, abs_tol=0.0)`, keyword-only) were hardcoded
        // to their defaults, completely ignoring whatever the caller
        // actually passed — `math.isclose(1.0, 1.0000001, rel_tol=1e-5)`
        // silently used `1e-9` instead, returning `False` for a
        // comparison that should clearly be `True`. Keyword args arrive
        // packed into a trailing dict per this codebase's own
        // `BuiltinFunction` calling convention.
        let mut rel_tol = 1e-9;
        let mut abs_tol = 0.0;
        if let Some(last) = args.last() {
            if let PyObject::Dict(kwargs) = &*last.borrow() {
                if let Ok(Some(v)) = kwargs.get(&py_str("rel_tol")) {
                    rel_tol = math_arg_f64(&v).ok_or_else(|| {
                        PyError::type_error("isclose() argument must be a number")
                    })?;
                }
                if let Ok(Some(v)) = kwargs.get(&py_str("abs_tol")) {
                    abs_tol = math_arg_f64(&v).ok_or_else(|| {
                        PyError::type_error("isclose() argument must be a number")
                    })?;
                }
            }
        }
        if rel_tol < 0.0 || abs_tol < 0.0 {
            return Err(PyError::value_error("tolerances must be non-negative"));
        }
        if a == b {
            return Ok(py_bool(true));
        }
        if a.is_infinite() || b.is_infinite() {
            return Ok(py_bool(false));
        }
        Ok(py_bool(
            (a - b).abs() <= (rel_tol * a.abs().max(b.abs())).max(abs_tol),
        ))
    });
    math_func!("gcd", |args| {
        let mut result = num_bigint::BigInt::from(0);
        for a in args {
            let v = math_int_value(a)
                .map_err(|_| PyError::type_error("gcd() arguments must be integers"))?;
            result = crate::object::bigint_gcd(&result, &v);
        }
        Ok(py_int(result))
    });
    math_func!("factorial", |args| {
        if args.len() != 1 {
            return Err(PyError::type_error(
                "factorial() takes exactly one argument",
            ));
        }
        let n = math_int_value(&args[0])
            .map_err(|_| PyError::type_error("factorial() argument must be an integer"))?;
        if n.sign() == num_bigint::Sign::Minus {
            return Err(PyError::value_error(
                "factorial() not defined for negative values",
            ));
        }
        if n > num_bigint::BigInt::from(i64::MAX) {
            return Err(PyError::overflow_error(
                "factorial() argument should not exceed 9223372036854775807",
            ));
        }
        let mut result = num_bigint::BigInt::from(1i64);
        let mut i = num_bigint::BigInt::from(2i64);
        while i <= n {
            result *= &i;
            i += 1;
        }
        Ok(py_int(result))
    });
    // `math.isqrt` was missing entirely (not even a stub) — real trigger:
    // CPython's own `test_math.testIsqrt`, which feeds it values up to
    // `2**200` and `10**5001`. Since those are real arbitrary-precision
    // bigints, this MUST use a proper bigint square root (`num_bigint`'s own
    // `BigInt::sqrt`, a Newton's-method implementation) rather than
    // converting to `f64` first (`f64::sqrt` silently loses precision far
    // below even `2**64`, and can't represent a 5001-digit input at all).
    math_func!("isqrt", |args| {
        if args.len() != 1 {
            return Err(PyError::type_error("isqrt() takes exactly one argument"));
        }
        let n = math_int_value(&args[0])
            .map_err(|_| PyError::type_error("isqrt() argument must be an integer"))?;
        if n.sign() == num_bigint::Sign::Minus {
            return Err(PyError::value_error("isqrt() argument must be nonnegative"));
        }
        Ok(py_int(n.sqrt()))
    });

    math_func!("comb", |args| {
        if args.len() != 2 {
            return Err(PyError::type_error("comb() takes exactly two arguments"));
        }
        let n = math_int_value(&args[0])
            .map_err(|_| PyError::type_error("comb() arguments must be integers"))?;
        let k = math_int_value(&args[1])
            .map_err(|_| PyError::type_error("comb() arguments must be integers"))?;
        if n.sign() == num_bigint::Sign::Minus {
            return Err(PyError::value_error("n must be a non-negative integer"));
        }
        if k.sign() == num_bigint::Sign::Minus {
            return Err(PyError::value_error("k must be a non-negative integer"));
        }
        if k > n {
            return Ok(py_int(0));
        }
        if k == num_bigint::BigInt::from(0) || &k == &n {
            return Ok(py_int(1));
        }
        let k = if &k * 2 > n { &n - &k } else { k };
        // A huge `k` means the result is astronomically large — cap it like
        // CPython's `math.comb` (OverflowError: result too large), instead
        // of looping ~2**999 times for `comb(2**1000, 2**999)`.
        if k > num_bigint::BigInt::from(1_000_000) {
            return Err(PyError::overflow_error(
                "result too large to be represented",
            ));
        }
        let k = k.to_u64().unwrap_or(u64::MAX) as i64;
        let mut result = num_bigint::BigInt::from(1);
        let mut i: i64 = 1;
        while i <= k {
            result = &result * (&n - i + 1) / i;
            i += 1;
        }
        Ok(py_int(result))
    });
    math_func!("perm", |args| {
        if args.len() < 1 || args.len() > 2 {
            return Err(PyError::type_error("perm() takes one or two arguments"));
        }
        let n = math_int_value(&args[0])
            .map_err(|_| PyError::type_error("perm() arguments must be integers"))?;
        if n.sign() == num_bigint::Sign::Minus {
            return Err(PyError::value_error("n must be a non-negative integer"));
        }
        let k = if args.len() == 2 {
            if matches!(&*args[1].borrow(), PyObject::None) {
                n.clone()
            } else {
                let k = math_int_value(&args[1])
                    .map_err(|_| PyError::type_error("perm() arguments must be integers"))?;
                if k.sign() == num_bigint::Sign::Minus {
                    return Err(PyError::value_error("k must be a non-negative integer"));
                }
                if k > n {
                    return Ok(py_int(0));
                }
                k
            }
        } else {
            n.clone()
        };
        // A huge `k` means the result is astronomically large — cap it like
        // CPython's `math.perm` (OverflowError), instead of looping ~2**1000
        // times for `perm(2**1000, 2**1000)`.
        if k > num_bigint::BigInt::from(1_000_000) {
            return Err(PyError::overflow_error(
                "result too large to be represented",
            ));
        }
        let k = k.to_u64().unwrap_or(u64::MAX) as i64;
        let mut result = num_bigint::BigInt::from(1);
        let mut i: i64 = 0;
        while i < k {
            result *= &n - i;
            i += 1;
        }
        Ok(py_int(result))
    });
    math_func!("lcm", |args| {
        fn lcm_big(a: &num_bigint::BigInt, b: &num_bigint::BigInt) -> num_bigint::BigInt {
            if a.sign() == num_bigint::Sign::NoSign || b.sign() == num_bigint::Sign::NoSign {
                return num_bigint::BigInt::from(0);
            }
            let g = crate::object::bigint_gcd(a, b);
            (a / &g) * b
        }
        let mut result = num_bigint::BigInt::from(1);
        for a in args {
            let v = math_int_value(a)
                .map_err(|_| PyError::type_error("lcm() arguments must be integers"))?;
            result = lcm_big(&result, &v);
        }
        // lcm is always non-negative (signs only come from the gcd's sign
        // convention).
        Ok(py_int(result.abs()))
    });
    math_func!("dist", |args| {
        if args.len() != 2 {
            return Err(PyError::type_error("dist() takes exactly two arguments"));
        }
        let iter_a = crate::object::builtin_iter(&[args[0].clone()])
            .map_err(|_| PyError::type_error("dist() argument must be iterable"))?;
        let iter_b = crate::object::builtin_iter(&[args[1].clone()])
            .map_err(|_| PyError::type_error("dist() argument must be iterable"))?;
        let mut sum_sq = 0.0f64;
        let mut max_abs = 0.0f64;
        let mut found_nan = false;
        let mut comps: Vec<(f64, f64)> = Vec::new();
        loop {
            let a = match crate::object::builtin_next(&[iter_a.clone()]) {
                Ok(v) => v,
                Err(PyError::StopIteration) => break,
                Err(e) => return Err(e),
            };
            let b = crate::object::builtin_next(&[iter_b.clone()])
                .map_err(|_| PyError::value_error("both arguments must be the same length"))?;
            let fa = match math_float_value(&a) {
                Ok(f) => {
                    if f.is_infinite() && matches!(&*a.borrow(), PyObject::Int(_)) {
                        return Err(PyError::overflow_error("int too large to convert to float"));
                    }
                    f
                }
                Err(e) => return Err(e),
            };
            let fb = match math_float_value(&b) {
                Ok(f) => {
                    if f.is_infinite() && matches!(&*b.borrow(), PyObject::Int(_)) {
                        return Err(PyError::overflow_error("int too large to convert to float"));
                    }
                    f
                }
                Err(e) => return Err(e),
            };
            comps.push((fa, fb));
            let diff = (fa - fb).abs();
            max_abs = max_abs.max(diff);
            found_nan |= diff.is_nan();
        }
        // `q` must be exhausted too (a longer `q` than `p` is a length mismatch).
        match crate::object::builtin_next(&[iter_b.clone()]) {
            Err(PyError::StopIteration) => {}
            _ => {
                return Err(PyError::value_error(
                    "both arguments must be the same length",
                ))
            }
        }
        if max_abs.is_infinite() {
            return Ok(py_float(f64::INFINITY));
        }
        if found_nan {
            return Ok(py_float(f64::NAN));
        }
        if max_abs == 0.0 {
            return Ok(py_float(0.0));
        }
        // Subnormal max (CPython's `max_e < -1023` branch): scale by DBL_MIN
        // so the diffs become normal before squaring.
        if max_abs < f64::MIN_POSITIVE {
            let mut sum_sq = 0.0;
            for (a, b) in &comps {
                let x = (a - b) / f64::MIN_POSITIVE;
                sum_sq += x * x;
            }
            return Ok(py_float(f64::MIN_POSITIVE * sum_sq.sqrt()));
        }
        // CPython's `vector_norm`: scale by a POWER OF TWO (from frexp of the
        // max coordinate), so the scaling is exact and `dist((14,1),(2,-4))`
        // comes out as exactly 13.0 (scaling by `max` itself would round).
        let max_e = max_abs.abs().log2().floor() as i32;
        let scale = 2f64.powi(-max_e);
        let mut sum_sq = 0.0;
        for (a, b) in &comps {
            let x = (a - b) * scale;
            sum_sq += x * x;
        }
        Ok(py_float(sum_sq.sqrt() / scale))
    });

    special::register_extra(&mut d);

    // ── Constants ─────────────────────────────────────────────────────────
    d.insert_str("pi", py_float(std::f64::consts::PI));
    d.insert_str("e", py_float(std::f64::consts::E));
    d.insert_str("tau", py_float(std::f64::consts::TAU));
    d.insert_str("inf", py_float(f64::INFINITY));
    d.insert_str("nan", py_float(f64::NAN));
    d
}
