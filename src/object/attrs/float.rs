// Auto-extracted from src/object/attrs/mod.rs lines 6650-6939
use crate::object::*;
use super::*;
use crate::interner;

pub(crate) fn get(o: &PyObject, name: &str) -> PyResult<PyObjectRef> {
    match o {
            PyObject::Float(_f) => {
                match name {
                    // `numbers.Complex` protocol every numeric type
                    // implements (a plain `float` is trivially its own real
                    // part with zero imaginary part and is its own
                    // conjugate) — entirely missing before, so any code
                    // written generically against that protocol (real
                    // trigger: CPython's own `test_abstract_numbers.py`)
                    // raised `AttributeError` on a plain `float`.
                    "real" => Ok(py_float(*_f)),
                    "imag" => Ok(py_float(0.0)),
                    "conjugate" => Ok(PyObjectRef::new(PyObject::BuiltinMethod {
                        name: "conjugate".to_string(),
                        func: |args| {
                            if let PyObject::Float(v) = &*args[0].borrow() {
                                Ok(py_float(*v))
                            } else {
                                Err(PyError::runtime_error("conjugate on non-float"))
                            }
                        },
                        self_obj: PyObjectRef::imm(PyObject::Float(*_f)),
                    })),
                    "__round__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__round__".to_string(),
                        func: |args| builtin_round(args),
                        self_obj: PyObjectRef::imm(PyObject::Float(*_f)),
                    })),
                    "__int__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__int__".to_string(),
                        func: |args| {
                            if let PyObject::Float(v) = &*args[0].borrow() {
                                Ok(py_int(*v as i64))
                            } else {
                                Err(PyError::runtime_error("__int__ on non-float"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    // Missing entirely before: `(2.0).__float__()` raised
                    // `AttributeError` even though `int.__float__` (above)
                    // was implemented — broke anything calling `.__float__()`
                    // generically on a value already known to be a float
                    // (e.g. `cmath`'s `test_input_type`: `f(arg.__float__())`).
                    "__float__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__float__".to_string(),
                        func: |args| {
                            if let PyObject::Float(v) = &*args[0].borrow() {
                                Ok(py_float(*v))
                            } else {
                                Err(PyError::runtime_error("__float__ on non-float"))
                            }
                        },
                        self_obj: PyObjectRef::imm(PyObject::Float(*_f)),
                    })),
                    // The numeric protocol dunders — real floats expose the
                    // full arithmetic operator set as methods
                    // (float(2).__truediv__(d), test_float's
                    // test_floatasratio calls exactly this). Route through
                    // the same py_* helpers the operators themselves use.
                    "__truediv__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: name.to_string(),
                        func: |args: &[PyObjectRef]| float_binop_dunder(args, false, 0),
                        self_obj: PyObjectRef::imm(PyObject::Float(*_f)),
                    })),
                    "__rtruediv__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: name.to_string(),
                        func: |args: &[PyObjectRef]| float_binop_dunder(args, true, 0),
                        self_obj: PyObjectRef::imm(PyObject::Float(*_f)),
                    })),
                    "__floordiv__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: name.to_string(),
                        func: |args: &[PyObjectRef]| float_binop_dunder(args, false, 1),
                        self_obj: PyObjectRef::imm(PyObject::Float(*_f)),
                    })),
                    "__rfloordiv__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: name.to_string(),
                        func: |args: &[PyObjectRef]| float_binop_dunder(args, true, 1),
                        self_obj: PyObjectRef::imm(PyObject::Float(*_f)),
                    })),
                    "__mod__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: name.to_string(),
                        func: |args: &[PyObjectRef]| float_binop_dunder(args, false, 2),
                        self_obj: PyObjectRef::imm(PyObject::Float(*_f)),
                    })),
                    "__rmod__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: name.to_string(),
                        func: |args: &[PyObjectRef]| float_binop_dunder(args, true, 2),
                        self_obj: PyObjectRef::imm(PyObject::Float(*_f)),
                    })),
                    "__pow__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: name.to_string(),
                        func: |args: &[PyObjectRef]| float_binop_dunder(args, false, 3),
                        self_obj: PyObjectRef::imm(PyObject::Float(*_f)),
                    })),
                    "__rpow__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: name.to_string(),
                        func: |args: &[PyObjectRef]| float_binop_dunder(args, true, 3),
                        self_obj: PyObjectRef::imm(PyObject::Float(*_f)),
                    })),
                    "__add__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: name.to_string(),
                        func: |args: &[PyObjectRef]| float_binop_dunder(args, false, 4),
                        self_obj: PyObjectRef::imm(PyObject::Float(*_f)),
                    })),
                    "__radd__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: name.to_string(),
                        func: |args: &[PyObjectRef]| float_binop_dunder(args, true, 4),
                        self_obj: PyObjectRef::imm(PyObject::Float(*_f)),
                    })),
                    "__sub__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: name.to_string(),
                        func: |args: &[PyObjectRef]| float_binop_dunder(args, false, 5),
                        self_obj: PyObjectRef::imm(PyObject::Float(*_f)),
                    })),
                    "__rsub__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: name.to_string(),
                        func: |args: &[PyObjectRef]| float_binop_dunder(args, true, 5),
                        self_obj: PyObjectRef::imm(PyObject::Float(*_f)),
                    })),
                    "__mul__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: name.to_string(),
                        func: |args: &[PyObjectRef]| float_binop_dunder(args, false, 6),
                        self_obj: PyObjectRef::imm(PyObject::Float(*_f)),
                    })),
                    "__rmul__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: name.to_string(),
                        func: |args: &[PyObjectRef]| float_binop_dunder(args, true, 6),
                        self_obj: PyObjectRef::imm(PyObject::Float(*_f)),
                    })),
                    "__neg__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__neg__".to_string(),
                        func: |args: &[PyObjectRef]| {
                            if args.is_empty() {
                                return Err(PyError::type_error("__neg__ needs 1 argument"));
                            }
                            crate::object::py_neg(&args[0])
                        },
                        self_obj: PyObjectRef::imm(PyObject::Float(*_f)),
                    })),
                    "as_integer_ratio" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "as_integer_ratio".to_string(),
                        func: |args| {
                            if let PyObject::Float(v) = &*args[0].borrow() {
                                let f = *v;
                                // CPython: inf -> OverflowError, nan -> ValueError.
                                if f.is_infinite() {
                                    return Err(PyError::overflow_error(
                                        "cannot convert Infinity to integer ratio",
                                    ));
                                }
                                if f.is_nan() {
                                    return Err(PyError::value_error(
                                        "cannot convert NaN to integer ratio",
                                    ));
                                }
                                // Decompose f64 into a reduced fraction
                                fn float_to_ratio(x: f64) -> (BigInt, BigInt) {
                                    if x == 0.0 {
                                        return (BigInt::from(0), BigInt::from(1));
                                    }
                                    let bits = x.to_bits();
                                    let sign = if (bits >> 63) == 0 { 1i64 } else { -1i64 };
                                    let biased_exp = ((bits >> 52) & 0x7ff) as i64;
                                    let mantissa = bits & 0x000f_ffff_ffff_ffff;
                                    if biased_exp == 0 {
                                        if mantissa == 0 {
                                            return (BigInt::from(0), BigInt::from(1));
                                        }
                                        // Subnormal: value = mantissa * 2^(-1074)
                                        let num = BigInt::from(sign) * BigInt::from(mantissa);
                                        let den = BigInt::from(1i64) << 1074;
                                        let g = gcd_bigint(&num, &den);
                                        (num / &g, den / g)
                                    } else {
                                        // Normal: add implicit leading 1
                                        let full_mantissa = 0x0010_0000_0000_0000 | mantissa;
                                        let exp = biased_exp - 1023 - 52;
                                        if exp >= 0 {
                                            (
                                                BigInt::from(sign)
                                                    * BigInt::from(full_mantissa)
                                                    * (BigInt::from(1i64) << (exp as u32)),
                                                BigInt::from(1),
                                            )
                                        } else {
                                            let num =
                                                BigInt::from(sign) * BigInt::from(full_mantissa);
                                            let den = BigInt::from(1i64) << ((-exp) as u32);
                                            let g = gcd_bigint(&num, &den);
                                            (num / &g, den / g)
                                        }
                                    }
                                }
                                fn gcd_bigint(a: &BigInt, b: &BigInt) -> BigInt {
                                    let mut a = a.clone();
                                    let mut b = b.clone();
                                    while !b.is_zero() {
                                        let t = b.clone();
                                        b = a % &t;
                                        a = t;
                                    }
                                    a.abs()
                                }
                                let (num, den) = float_to_ratio(f);
                                Ok(py_tuple(vec![py_int(num), py_int(den)]))
                            } else {
                                Err(PyError::runtime_error("as_integer_ratio on non-float"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "hex" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "hex".to_string(),
                        func: |args| {
                            if let PyObject::Float(v) = &*args[0].borrow() {
                                let bits = v.to_bits();
                                let sign = if (bits >> 63) != 0 { "-" } else { "" };
                                let biased_exp = ((bits >> 52) & 0x7ff) as i64;
                                let mantissa = bits & 0x000f_ffff_ffff_ffff;
                                if biased_exp == 0x7ff {
                                    if mantissa == 0 {
                                        Ok(py_str(&format!("{}inf", sign)))
                                    } else {
                                        Ok(py_str(&format!("{}nan", sign)))
                                    }
                                } else if *v == 0.0 {
                                    Ok(py_str(&format!("{}0x0.0p+0", sign)))
                                } else {
                                    let hex_mantissa = format!("{:013x}", mantissa);
                                    if biased_exp == 0 {
                                        // Subnormal: CPython writes the raw
                                        // 52-bit mantissa after a 0x0. prefix
                                        // at fixed exponent -1022
                                        // ('0x0.048bd262b030bp-1022'), not the
                                        // normalized 0x1.XXXXp-1023 form.
                                        Ok(py_str(&format!("{}0x0.{}p-1022", sign, hex_mantissa)))
                                    } else {
                                        let exp = biased_exp - 1023;
                                        // CPython keeps ALL 13 frac hex digits
                                        // (52 mantissa bits); trimming trailing
                                        // zeros produced a different hex string
                                        // than float.hex()/test_strtod expect
                                        // (e.g. '0x1.6544243f809b0p+54' not
                                        // '0x1.6544243f809bp+54').
                                        Ok(py_str(&format!(
                                            "{}0x1.{}p{:+}",
                                            sign, hex_mantissa, exp
                                        )))
                                    }
                                }
                            } else {
                                Err(PyError::runtime_error("hex on non-float"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "is_integer" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "is_integer".to_string(),
                        func: |args| {
                            if let PyObject::Float(v) = &*args[0].borrow() {
                                Ok(py_bool(v.fract() == 0.0))
                            } else {
                                Err(PyError::runtime_error("is_integer on non-float"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__ceil__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__ceil__".to_string(),
                        func: |args| {
                            if let PyObject::Float(v) = &*args[0].borrow() {
                                f64_to_int_ceil_floor_trunc(*v, 1).map(py_int)
                            } else {
                                Err(PyError::runtime_error("__ceil__ on non-float"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__floor__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__floor__".to_string(),
                        func: |args| {
                            if let PyObject::Float(v) = &*args[0].borrow() {
                                f64_to_int_ceil_floor_trunc(*v, 2).map(py_int)
                            } else {
                                Err(PyError::runtime_error("__floor__ on non-float"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    "__trunc__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__trunc__".to_string(),
                        func: |args| {
                            if let PyObject::Float(v) = &*args[0].borrow() {
                                f64_to_int_ceil_floor_trunc(*v, 0).map(py_int)
                            } else {
                                Err(PyError::runtime_error("__trunc__ on non-float"))
                            }
                        },
                        self_obj: PyObjectRef::new(PyObject::None),
                    })),
                    _ => Err(PyError::attribute_error(format!(
                        "'float' object has no attribute '{}'",
                        name
                    ))),
                }
            }
        _ => unreachable!("{} called with non-{} object", stringify!(get), o.type_name()),
    }
}
