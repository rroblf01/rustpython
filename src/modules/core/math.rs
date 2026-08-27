use crate::object::*;
use std::collections::HashMap;
use std::sync::atomic::AtomicI64;
use num_traits::{Signed, ToPrimitive};
use std::rc::Rc;


/// Like `PyObjectRef::as_f64()`, but also consults `__float__` for an
/// `Instance` that isn't a native numeric type — real Python's `math`
/// functions all accept ANY object implementing `__float__` (e.g. custom
/// numeric-like classes, `decimal.Decimal`, `fractions.Fraction`), not just
/// literal `int`/`float`. Most of `math`'s own native functions previously
/// used bare `.as_f64()` directly, which only ever handles native
/// int/float/bool — rejecting a perfectly valid `__float__`-defining object
/// with a spurious `TypeError`. Found via CPython's own `test_math.py`
/// (`hypot(0.75, FloatLike(-1.))` and similar for `isclose`/`isnan`/
/// `copysign`/`fmod`/`atan2`/`dist`/`sumprod`).
/// Integer value of a `math` integer argument: a native int, an int-subclass
/// instance (its int backing), or any `__index__` object.
fn math_int_value(v: &PyObjectRef) -> PyResult<num_bigint::BigInt> {
    if let Some(n) = crate::object::int_value_or_backing(v) {
        return Ok(n);
    }
    crate::object::to_index(v)
}

/// `math` float argument (native float/int, or any `__float__` object —
/// descriptors resolved so a raising `__float__` descriptor propagates).
fn math_float_value(v: &PyObjectRef) -> PyResult<f64> {
    if let Some(f) = v.as_f64() {
        return Ok(f);
    }
    let typ = if let PyObject::Instance { typ, .. } = &*v.borrow() {
        typ.clone()
    } else {
        return Err(PyError::type_error("argument must be a number"));
    };
    let Some(f) = lookup_dunder_via_mro(&typ, "__float__") else {
        return Err(PyError::type_error("argument must be a number"));
    };
    let has_get = f.borrow().get_attribute("__get__").is_ok();
    if has_get {
        // Descriptor protocol: `f.__get__(instance, type)`.
        let get = f.borrow().get_attribute("__get__").unwrap();
        let resolved = call_bound_method(get, f.clone(), vec![v.clone(), typ.clone()])?;
        let inner = resolved
            .as_f64()
            .ok_or_else(|| PyError::type_error("__float__ returned non-float"))?;
        return Ok(inner);
    }
    let result = call_bound_method(f, v.clone(), vec![])?;
    result
        .as_f64()
        .ok_or_else(|| PyError::type_error("__float__ returned non-float"))
}

fn math_arg_f64(v: &PyObjectRef) -> Option<f64> {
    if let Some(f) = v.as_f64() {
        return Some(f);
    }
    // Float (and int) subclasses like MyFloat(float) store the real
    // numeric value as a native backing (e.g. MyFloat(NaN)). `as_f64()`
    // returns None for Instances, so check the backing directly.
    if let Some(backing) = crate::object::native_backing_of(v) {
        if let Some(f) = backing.as_f64() {
            return Some(f);
        }
    }
    let f = {
        let typ = if let PyObject::Instance { typ, .. } = &*v.borrow() {
            Some(typ.clone())
        } else {
            None
        }?;
        lookup_dunder_via_mro(&typ, "__float__")?
    };
    call_bound_method(f, v.clone(), vec![]).ok()?.as_f64()
}

/// `math.floor`/`math.ceil`/`math.trunc` dispatch to `__floor__`/`__ceil__`/
/// `__trunc__` on an instance when present (a Python `Function` method, a
/// native `BuiltinFunction`/`BuiltinMethod`, or `None` = explicitly
/// disabled, which must raise TypeError rather than fall through). Returns
/// `Ok(None)` when no usable dunder exists.
fn math_call_int_dunder(self_obj: &PyObjectRef, name: &str) -> PyResult<Option<PyObjectRef>> {
    let typ = if let PyObject::Instance { typ, .. } = &*self_obj.borrow() {
        typ.clone()
    } else {
        return Ok(None);
    };
    let Some(f) = lookup_dunder_via_mro(&typ, name) else {
        return Ok(None);
    };
    // Generic descriptor protocol (mirrors a real instance attribute
    // access): if the found dunder value is a descriptor — an arbitrary
    // object with `__get__`, e.g. `test_math`'s `BadDescr` — invoke
    // `__get__(obj, type)` and dispatch on its result. CPython propagates
    // whatever `__get__` raises (BadDescr raises `ValueError`) instead of
    // trying to call the raw, non-callable value. Plain functions/methods
    // also carry `__get__`; invoking it here returns the same bound
    // callable the arms below would have dispatched on anyway.
    let f = {
        let get_result = f.borrow().get_attribute("__get__");
        match get_result {
            Ok(get_fn) => crate::object::call_function_disposable(
                &get_fn,
                vec![self_obj.clone(), typ.clone()],
                vec![],
            )?,
            Err(_) => f,
        }
    };
    let b = f.borrow();
    match &*b {
        PyObject::None => Err(PyError::type_error(format!(
            "'{}' object does not support {}",
            crate::object::get_type_name_for_instance(&typ),
            name
        ))),
        PyObject::BuiltinFunction { func, .. } => {
            let func = *func;
            drop(b);
            Ok(Some(func(&[self_obj.clone()])?))
        }
        PyObject::BuiltinMethod { func, .. } => {
            let func = *func;
            drop(b);
            Ok(Some(func(&[self_obj.clone()])?))
        }
        PyObject::BoundMethod { .. } => {
            drop(b);
            Ok(Some(call_bound_method(f, self_obj.clone(), vec![])?))
        }
        _ => {
            drop(b);
            Ok(Some(call_bound_method(f, self_obj.clone(), vec![])?))
        }
    }
}

/// Exact summation of `items` — the "lsum" algorithm behind CPython's
/// `math.fsum` and `math.sumprod`'s float path: each float is split with
/// frexp into (mantissa, exponent), mantissas are aligned and added as big
/// integers, and the exact result is rounded once at the end, so
/// cancellation can never destroy small terms. Handles NaN / ±inf /
/// overflow / underflow the way CPython's fsum does.
fn exact_fsum(items: &[f64]) -> PyResult<f64> {
    let mant_dig = 53i64;
    let mut tmant = num_bigint::BigInt::from(0);
    let mut texp: i64 = 0;
    let mut seen = false;
    let mut pos_inf = false;
    let mut neg_inf = false;
    for &x in items {
        if x.is_nan() {
            return Ok(f64::NAN);
        }
        if x.is_infinite() {
            if x > 0.0 {
                pos_inf = true;
            } else {
                neg_inf = true;
            }
            continue;
        }
        if x == 0.0 {
            continue;
        }
        // frexp: x = mant * 2^exp with mant in [0.5, 1)
        let bits = x.to_bits();
        let sign = if bits >> 63 == 0 { 1.0 } else { -1.0 };
        let biased = ((bits >> 52) & 0x7ff) as i64;
        let (mant_mag, exp) = if biased == 0 {
            (f64::from_bits(bits & 0x000f_ffff_ffff_ffff), -1022)
        } else {
            (
                f64::from_bits((bits & 0x000f_ffff_ffff_ffff) | 0x3fe0_0000_0000_0000),
                biased - 1022,
            )
        };
        let mant_i = num_bigint::BigInt::from(crate::object::ldexp_f64(
            sign * mant_mag,
            mant_dig as i32,
        ) as i64);
        let exp = exp - mant_dig;
        if !seen {
            tmant = mant_i;
            texp = exp;
            seen = true;
        } else if texp > exp {
            tmant <<= (texp - exp) as usize;
            texp = exp;
            tmant += mant_i;
        } else {
            tmant += mant_i << ((exp - texp) as usize);
        }
    }
    if pos_inf && neg_inf {
        return Err(PyError::value_error("math domain error"));
    }
    if pos_inf {
        return Ok(f64::INFINITY);
    }
    if neg_inf {
        return Ok(f64::NEG_INFINITY);
    }
    if !seen {
        return Ok(0.0);
    }
    // Round the exact integer result once to a double (round-half-to-even,
    // trimming to 53 significant bits), then scale by the exact power of
    // two 2^texp.
    let neg = tmant.sign() == num_bigint::Sign::Minus;
    let mag = tmant.abs();
    let bits = mag.bits() as i64;
    let etiny = -1074i64; // sys.float_info.min_exp - mant_dig
    let tail = (bits - mant_dig).max(etiny - texp);
    let mut texp_final = texp;
    let m = if tail > 0 {
        let h = num_bigint::BigInt::from(1u64) << ((tail - 1) as usize);
        let two_h = &h << 1;
        let q = &mag / &two_h;
        let half = (&mag & &h) != num_bigint::BigInt::from(0);
        let three_h_minus_1 = &(&(&h << 1) + &h) - 1u32;
        let sticky = (&mag & &three_h_minus_1) != num_bigint::BigInt::from(0);
        texp_final += tail;
        q + (if half && sticky { 1u32 } else { 0u32 })
    } else {
        mag
    };
    let m = if neg { -m } else { m };
    let result = m.to_f64().unwrap_or(0.0);
    let scaled = crate::object::ldexp_f64(result, texp_final.clamp(-2000, 2000) as i32);
    if scaled.is_infinite() {
        return Err(PyError::overflow_error("math range error"));
    }
    Ok(scaled)
}

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
    // Integer -> (m, e) frexp split for logarithms: `n = m * 2^e` with
    // m in [0.5, 1), so log(n) = ln(m) + e*ln(2) — computed exactly even
    // for ints far too large for f64 (log(10**1000) must not be +inf).
    fn log_frexp_int(n: &num_bigint::BigInt) -> (f64, f64) {
        let d = n.bits();
        let e = d as f64;
        let m = if d > 53 {
            let top = (n >> (d - 53)).to_u64().unwrap_or(0) as f64;
            top / 9007199254740992.0
        } else {
            n.to_f64().unwrap_or(0.0) / 2f64.powi(d as i32)
        };
        (m, e)
    }
    fn math_log2_value(v: &PyObjectRef) -> PyResult<f64> {
        let b = v.borrow();
        if let PyObject::Int(i) = &*b {
            if i.sign() == num_bigint::Sign::NoSign || i.sign() == num_bigint::Sign::Minus {
                return Err(PyError::value_error("expected a positive input"));
            }
            let (m, e) = log_frexp_int(i);
            return Ok(m.log2() + e);
        }
        let x = math_arg_f64(v).ok_or_else(|| PyError::type_error("a float is required"))?;
        if x <= 0.0 {
            return Err(PyError::value_error(format!(
                "expected a positive input, got {:?}",
                x
            )));
        }
        Ok(x.log2())
    }
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
    // CPython's `vector_norm` (faithfully rounded hypot): exact squaring
    // (Dekker two-product), compensated Neumaier-style summation, and a
    // square-root differential correction so the result is within 1/2 ulp
    // of the correctly rounded hypotenuse.
    fn dl_fast_sum(a: f64, b: f64) -> (f64, f64) {
        let x = a + b;
        let y = (a - x) + b;
        (x, y)
    }
    fn vector_norm(vals: &[f64], max: f64) -> f64 {
        if max == 0.0 || vals.len() <= 1 {
            return max;
        }
        // frexp exponent of max (max = m * 2^max_e, m in [0.5, 1))
        let max_bits = max.to_bits();
        let max_e = (((max_bits >> 52) & 0x7ff) as i32) - 1022;
        if max_e < -1023 {
            // max is subnormal: scale up, recurse, scale back.
            let scaled: Vec<f64> = vals.iter().map(|v| v / f64::MIN_POSITIVE).collect();
            return f64::MIN_POSITIVE * vector_norm(&scaled, max / f64::MIN_POSITIVE);
        }
        let scale = crate::object::ldexp_f64(1.0, -max_e);
        let mut csum = 1.0f64;
        let mut frac1 = 0.0f64;
        let mut frac2 = 0.0f64;
        for v in vals {
            let x = v * scale; // lossless scaling; |x| < 1
            let (pr_hi, pr_lo) = dl_mul(x, x); // exact squaring
            let (sm_hi, sm_lo) = dl_fast_sum(csum, pr_hi); // |csum| >= |pr_hi|
            csum = sm_hi;
            frac1 += pr_lo;
            frac2 += sm_lo;
        }
        let mut h = (csum - 1.0 + (frac1 + frac2)).sqrt();
        // Differential correction: h ~= sqrt(h^2 + x) ~= h + x/(2h).
        let (pr_hi, pr_lo) = dl_mul(-h, h);
        let (sm_hi, sm_lo) = dl_fast_sum(csum, pr_hi);
        csum = sm_hi;
        frac1 += pr_lo;
        frac2 += sm_lo;
        let x = csum - 1.0 + (frac1 + frac2);
        h += x / (2.0 * h);
        h / scale
    }
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

    // Additional math functions
    math_func!("ldexp", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("ldexp() requires 2 arguments"));
        }
        let x = math_arg_f64(&args[0])
            .ok_or_else(|| PyError::type_error("argument must be a number"))?;
        let exp_big = math_int_value(&args[1])
            .map_err(|_| PyError::type_error("exponent must be an integer"))?;
        let exp = exp_big
            .to_i64()
            .map(|e| e.clamp(i32::MIN as i64, i32::MAX as i64))
            .unwrap_or_else(|| {
                // Exponent too big to fit i64: saturate to the extreme
                // (10**40 -> huge positive/negative -> inf/0.0).
                if exp_big.sign() == num_bigint::Sign::Minus {
                    i32::MIN as i64
                } else {
                    i32::MAX as i64
                }
            });
        let result = crate::object::ldexp_f64(x, exp as i32);
        if result.is_infinite() && x.is_finite() {
            return Err(PyError::overflow_error("math range error"));
        }
        Ok(py_float(result))
    });
    math_func!("fsum", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("fsum() requires an argument"));
        }
        // Any iterable (generator, map/filter, custom __iter__ ...).
        let items = collect_iterable(&args[0])?;
        let mut vals = Vec::with_capacity(items.len());
        for item in &items {
            let x = math_arg_f64(&item).ok_or_else(|| {
                PyError::type_error(format!(
                    "must be real number, not {}",
                    item.borrow().type_name()
                ))
            })?;
            // A huge INT that overflows to +inf (e.g. 10**1000) is an
            // OverflowError; a genuine float inf is handled by exact_fsum.
            if x.is_infinite() && matches!(&*item.borrow(), PyObject::Int(_)) {
                return Err(PyError::overflow_error("int too large to convert to float"));
            }
            vals.push(x);
        }
        Ok(py_float(exact_fsum(&vals)?))
    });
    // TripleLength fused multiply-add (CPython's `tl_fma`, Algorithm 5.10
    // with SumKVert for K=3): a ~106-bit-exact accumulator for
    // `math.sumprod`'s float path. Like CPython, this is deliberately a
    // LITTLE less accurate than fully exact summation — a tiny term
    // alongside a huge one can be lost in the two-sum (the -7.5 in
    // sumprod((-5,-5,10), (1.5, 2**62, 2**61)) vanishes, giving 0.0),
    // which CPython's own test suite pins down.
    fn dl_sum(a: f64, b: f64) -> (f64, f64) {
        // Algorithm 3.1 (error-free transformation of a sum)
        let x = a + b;
        let z = x - a;
        let y = (a - (x - z)) + (b - z);
        (x, y)
    }
    fn dl_mul(a: f64, b: f64) -> (f64, f64) {
        let hi = a * b;
        let lo = a.mul_add(b, -hi);
        (hi, lo)
    }
    fn tl_fma(x: f64, y: f64, total: (f64, f64, f64)) -> (f64, f64, f64) {
        let (pr_hi, pr_lo) = dl_mul(x, y);
        let (sm_hi, sm_lo) = dl_sum(total.0, pr_hi);
        let (r1_hi, r1_lo) = dl_sum(total.1, pr_lo);
        let (r2_hi, r2_lo) = dl_sum(r1_hi, sm_lo);
        (sm_hi, r2_hi, total.2 + r1_lo + r2_lo)
    }
    fn tl_to_d(total: (f64, f64, f64)) -> f64 {
        let (last_hi, last_lo) = dl_sum(total.1, total.0);
        total.2 + last_lo + last_hi
    }

    // sumprod(p, q) — dot product of two equal-length iterables (added in
    // CPython 3.12), needed by real CPython's own `Lib/statistics.py`.
    math_func!("sumprod", |args| {
        if args.len() != 2 {
            return Err(PyError::type_error(
                "sumprod() takes exactly 2 positional arguments",
            ));
        }
        let p = collect_iterable(&args[0])?;
        let q = collect_iterable(&args[1])?;
        if p.len() != q.len() {
            return Err(PyError::value_error("inputs are not the same length"));
        }
        // Faithful port of CPython's math_sumprod_impl: three lanes.
        //  - int lane: exact products of two exact ints, accumulated in a
        //    C-long-sized integer; on overflow (or a non-int pair) it is
        //    finalized and disabled FOREVER.
        //  - float lane: float/int/bool pairs through the TripleLength
        //    accumulator (tl_fma); on a non-finite result or a non-float
        //    element it is finalized and disabled forever.
        //  - normal lane: real object `*` and `+` for whatever remains
        //    (Fraction/Decimal keep exact types; huge int x float raises
        //    OverflowError; inf + -inf is NaN).
        let as_i64 = |v: &PyObjectRef| -> Option<i64> {
            if let PyObject::Int(i) = &*v.borrow() {
                i.to_i64()
            } else {
                None
            }
        };
        let floatable = |v: &PyObjectRef| -> Option<f64> {
            match &*v.borrow() {
                PyObject::Int(i) => i.to_f64().filter(|f| f.is_finite()),
                _ => v.as_f64(),
            }
        };
        let mut total = py_int(0);
        let mut int_enabled = true;
        let mut int_total: i64 = 0;
        let mut int_in_use = false;
        let mut flt_enabled = true;
        let mut flt_total: (f64, f64, f64) = (0.0, 0.0, 0.0);
        let mut flt_in_use = false;
        for (a, b) in p.iter().zip(q.iter()) {
            let both_int = matches!(&*a.borrow(), PyObject::Int(_))
                && matches!(&*b.borrow(), PyObject::Int(_));
            if int_enabled {
                let prod = if both_int {
                    as_i64(a)
                        .zip(as_i64(b))
                        .and_then(|(ai, bi)| ai.checked_mul(bi))
                } else {
                    None
                };
                if let Some(prod) = prod {
                    if let Some(nxt) = int_total.checked_add(prod) {
                        int_total = nxt;
                        int_in_use = true;
                        continue;
                    }
                }
                // finalize int lane
                int_enabled = false;
                if int_in_use {
                    total = crate::object::py_add(&total, &py_int(int_total))?;
                    int_total = 0;
                    int_in_use = false;
                }
            }
            if flt_enabled {
                // CPython's float lane requires at least one exact FLOAT
                // operand (float*float, float*int, int*float); a pure
                // int*int pair never enters it, even after the int lane
                // overflowed (that's exactly how the -7.5 in
                // sumprod((-5,-5,10), (1.5, 2**62, 2**61)) gets lost: the
                // int products overflow a C long, flushing the float lane
                // and falling to ordinary float arithmetic).
                let p_is_float = matches!(&*a.borrow(), PyObject::Float(_));
                let q_is_float = matches!(&*b.borrow(), PyObject::Float(_));
                let nft = if p_is_float || q_is_float {
                    match (floatable(a), floatable(b)) {
                        (Some(fa), Some(fb)) => tl_fma(fa, fb, flt_total),
                        _ => (f64::NAN, 0.0, 0.0),
                    }
                } else {
                    (f64::NAN, 0.0, 0.0)
                };
                if nft.0.is_finite() {
                    flt_total = nft;
                    flt_in_use = true;
                    continue;
                }
                // finalize float lane
                flt_enabled = false;
                if flt_in_use {
                    total = crate::object::py_add(&total, &py_float(tl_to_d(flt_total)))?;
                    flt_total = (0.0, 0.0, 0.0);
                    flt_in_use = false;
                }
            }
            // normal lane
            let term = crate::object::py_mul(a, b)?;
            total = crate::object::py_add(&total, &term)?;
        }
        if int_in_use {
            total = crate::object::py_add(&total, &py_int(int_total))?;
        }
        if flt_in_use {
            total = crate::object::py_add(&total, &py_float(tl_to_d(flt_total)))?;
        }
        Ok(total)
    });
    math_func!("remainder", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("remainder() requires 2 arguments"));
        }
        let x = math_arg_f64(&args[0])
            .ok_or_else(|| PyError::type_error("argument must be a number"))?;
        let y = math_arg_f64(&args[1])
            .ok_or_else(|| PyError::type_error("argument must be a number"))?;
        if x.is_nan() {
            return Ok(py_float(f64::NAN));
        }
        if y.is_nan() {
            return Ok(py_float(f64::NAN));
        }
        if x.is_infinite() {
            return Err(PyError::value_error("math domain error"));
        }
        if y == 0.0 {
            return Err(PyError::value_error("math domain error"));
        }
        if y.is_infinite() {
            return Ok(py_float(x));
        }
        // Faithful port of CPython's m_remainder: reduce |x| mod |y| via
        // fmod (never overflows), compare against the complement c =
        // absy - m (Sterbenz-exact), and on an exact half choose the even
        // quotient. All steps stay within [0, |y|], so huge quotients
        // can't overflow the intermediate `y * round(x/y)`.
        let absx = x.abs();
        let absy = y.abs();
        let m = (absx % absy).abs();
        let c = absy - m;
        let r = if m < c {
            m
        } else if m > c {
            -c
        } else {
            m - 2.0 * ((0.5 * (absx - m)) % absy).abs()
        };
        Ok(py_float(f64::copysign(1.0, x) * r))
    });
    math_func!("modf", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("modf() requires an argument"));
        }
        let x = math_arg_f64(&args[0])
            .ok_or_else(|| PyError::type_error("argument must be a number"))?;
        if x.is_infinite() {
            return Ok(py_tuple(vec![py_float(f64::copysign(0.0, x)), py_float(x)]));
        }
        if x.is_nan() {
            return Ok(py_tuple(vec![py_float(f64::NAN), py_float(f64::NAN)]));
        }
        let frac = x.fract();
        let integer = x.trunc();
        Ok(py_tuple(vec![py_float(frac), py_float(integer)]))
    });
    math_func!("frexp", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("frexp() requires an argument"));
        }
        let x = math_arg_f64(&args[0])
            .ok_or_else(|| PyError::type_error("argument must be a number"))?;
        if x == 0.0 {
            return Ok(py_tuple(vec![py_float(0.0), py_int(0)]));
        }
        if x.is_infinite() || x.is_nan() {
            return Ok(py_tuple(vec![py_float(x), py_int(0)]));
        }
        let bits = x.to_bits();
        let biased_exp = ((bits >> 52) & 0x7ff) as i64;
        let normalized_exp = biased_exp - 1023;
        let mantissa_bits = bits & 0x000f_ffff_ffff_ffff;
        let sign_bit = bits & 0x8000_0000_0000_0000;
        // Reconstruct mantissa in range [0.5, 1)
        let fraction_bits = sign_bit | (0x3fe << 52) | mantissa_bits;
        let fraction = f64::from_bits(fraction_bits);
        // x = fraction * 2^exp with fraction in [0.5, 1): the fraction
        // reconstruction above divides by 2 (exponent 0x3fe = 1022), so
        // the reported exponent is biased_exp - 1023 + 1.
        Ok(py_tuple(vec![
            py_float(fraction),
            py_int(normalized_exp + 1),
        ]))
    });
    math_func!("ulp", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("ulp() requires an argument"));
        }
        let x = math_arg_f64(&args[0])
            .ok_or_else(|| PyError::type_error("argument must be a number"))?;
        // Calculate ULP: distance to next representable float
        if x.is_nan() {
            return Ok(py_float(f64::NAN));
        }
        if x.is_infinite() {
            return Ok(py_float(f64::INFINITY));
        }
        if x == 0.0 {
            // ulp(±0.0) is the smallest subnormal (CPython: 5e-324)
            return Ok(py_float(f64::from_bits(1)));
        }
        let abs = x.abs();
        // One ulp above `abs`; at the very top of the exponent range that
        // overflows to +inf (ulp(FLOAT_MAX) must still be the binade gap),
        // so measure downward instead.
        let up = f64::from_bits(abs.to_bits() + 1);
        let result = if up.is_infinite() {
            abs - f64::from_bits(abs.to_bits() - 1)
        } else {
            up - abs
        };
        Ok(py_float(result))
    });
    math_func!("nextafter", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("nextafter() requires 2 arguments"));
        }
        let x = math_arg_f64(&args[0])
            .ok_or_else(|| PyError::type_error("argument must be a number"))?;
        let y = math_arg_f64(&args[1])
            .ok_or_else(|| PyError::type_error("argument must be a number"))?;
        let steps = if args.len() >= 3 {
            let step_arg = &args[2];
            if let PyObject::Dict(kwargs) = &*step_arg.borrow() {
                match kwargs.get(&py_str("steps")) {
                    Ok(Some(v)) => math_int_value(&v)
                        .map_err(|_| PyError::type_error("steps argument must be an integer"))?
                        .to_i64()
                        .ok_or_else(|| PyError::overflow_error("steps too large"))?,
                    _ => 1,
                }
            } else {
                math_int_value(step_arg)
                    .map_err(|_| PyError::type_error("steps argument must be an integer"))?
                    .to_i64()
                    .ok_or_else(|| PyError::overflow_error("steps too large"))?
            }
        } else {
            1
        };
        if x.is_nan() || y.is_nan() {
            return Ok(py_float(f64::NAN));
        }
        if steps < 0 {
            return Err(PyError::value_error("steps must not be negative"));
        }
        if x == y {
            // CPython returns `y` (this also handles the -0.0 -> +0.0
            // crossing, where bit-level equality differs from numeric).
            return Ok(py_float(y));
        }
        // Monotonic signed ordering of IEEE-754 bit patterns: negative
        // floats map below 0 (i64::MIN - bits-as-i64), non-negative map
        // directly, so +/-inf are the range bounds and stepping the signed
        // value by one moves exactly one ulp toward y (correct across the
        // sign boundary and at ±0, unlike the naive bits+1/bits-1 trick).
        let to_ord = |bits: u64| -> i64 {
            let i = bits as i64;
            if i >= 0 {
                i
            } else {
                i64::MIN - i
            }
        };
        let from_ord = |o: i64| -> u64 {
            let i = if o >= 0 { o } else { i64::MIN - o };
            i as u64
        };
        let ord_neg_inf = to_ord(0xfff0_0000_0000_0000u64);
        let ord_pos_inf = to_ord(0x7ff0_0000_0000_0000u64);
        let dir: i64 = if y > x { 1 } else { -1 };
        let target = (to_ord(x.to_bits()) as i128) + (dir as i128) * (steps as i128);
        let target = target.clamp(ord_neg_inf as i128, ord_pos_inf as i128);
        if target == 0 && x.to_bits() != 0 {
            // Landing exactly on the zero boundary (a subnormal stepping
            // toward 0): preserve x's sign — CPython returns ±0.0 matching
            // the side x came from.
            return Ok(py_float(f64::copysign(0.0, x)));
        }
        Ok(py_float(f64::from_bits(from_ord(target as i64))))
    });
    math_func!("prod", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("prod() requires an argument"));
        }
        let start = if args.len() > 1 {
            match &*args[1].borrow() {
                PyObject::Dict(kwargs) => match kwargs.get(&py_str("start")) {
                    Ok(Some(v)) => v.clone(),
                    _ => py_int(1),
                },
                // start is keyword-only; a second POSITIONAL arg is an error.
                _ => {
                    return Err(PyError::type_error(
                        "prod() takes at most 1 positional argument",
                    ))
                }
            }
        } else {
            py_int(1)
        };
        let items = collect_iterable(&args[0])?;
        // prod is plain repeated multiplication, so delegate to the real
        // object `*` (handles int/float/Fraction/Decimal and propagates
        // any error from user __mul__/__rmul__, e.g. RuntimeError).
        let mut result = start;
        for item in &items {
            result = crate::object::py_mul(&result, item)?;
        }
        Ok(result)
    });

    // ── Constants ─────────────────────────────────────────────────────────
    d.insert_str("pi", py_float(std::f64::consts::PI));
    d.insert_str("e", py_float(std::f64::consts::E));
    d.insert_str("tau", py_float(std::f64::consts::TAU));
    d.insert_str("inf", py_float(f64::INFINITY));
    d.insert_str("nan", py_float(f64::NAN));
    d
}
