use crate::object::*;
use std::collections::HashMap;
use num_bigint::{BigInt, Sign};
use num_traits::{One, Signed, Zero};
use num_traits::ToPrimitive;
use std::rc::Rc;
use crate::modules::data::decimal::*;
use crate::modules::data::fractions::*;
/// Format a Fraction exactly for float-style presentation types — port of
/// CPython's `Fraction._format_float_style`. `den` must be positive.
pub fn frac_format_exact(num: BigInt, den: BigInt, spec: &str) -> PyResult<String> {
    let chars: Vec<char> = spec.chars().collect();
    let len = chars.len();
    let mut idx = 0;
    let mut fill = ' ';
    let mut align = '>';
    let mut align_explicit = false;
    if idx + 1 < len && matches!(chars[idx + 1], '<' | '>' | '^' | '=') {
        fill = chars[idx];
        align = chars[idx + 1];
        align_explicit = true;
        idx += 2;
    } else if idx < len && matches!(chars[idx], '<' | '>' | '^' | '=') {
        align = chars[idx];
        align_explicit = true;
        idx += 1;
    }
    let mut sign = '-';
    if idx < len && matches!(chars[idx], '+' | '-' | ' ') {
        sign = chars[idx];
        idx += 1;
    }
    let mut no_neg_zero = false;
    if idx < len && chars[idx] == 'z' {
        no_neg_zero = true;
        idx += 1;
    }
    let mut alt = false;
    if idx < len && chars[idx] == '#' {
        alt = true;
        idx += 1;
    }
    let mut zeropad = false;
    if idx < len && chars[idx] == '0' && idx + 1 < len && chars[idx + 1].is_ascii_digit() {
        zeropad = true;
        idx += 1;
    }
    let mut width = 0usize;
    while idx < len && chars[idx].is_ascii_digit() {
        width = width * 10 + chars[idx].to_digit(10).unwrap() as usize;
        idx += 1;
    }
    let mut int_sep: Option<char> = None;
    if idx < len && (chars[idx] == ',' || chars[idx] == '_') {
        int_sep = Some(chars[idx]);
        idx += 1;
    }
    let mut precision = 6usize;
    let mut frac_sep: Option<char> = None;
    if idx < len && chars[idx] == '.' {
        idx += 1;
        // CPython requires the '.' to be followed by a digit or separator
        // ('.e', '.f' with no precision are invalid).
        if idx >= len || !(chars[idx].is_ascii_digit() || chars[idx] == ',' || chars[idx] == '_') {
            return Err(PyError::value_error(format!(
                "Invalid format specifier '{}' for object of type 'Fraction'",
                spec
            )));
        }
        let start = idx;
        while idx < len && chars[idx].is_ascii_digit() {
            idx += 1;
        }
        if idx > start {
            precision = chars[start..idx]
                .iter()
                .collect::<String>()
                .parse()
                .unwrap_or(6);
        }
        if idx < len && (chars[idx] == ',' || chars[idx] == '_') {
            frac_sep = Some(chars[idx]);
            idx += 1;
        }
    }
    let ptype = if idx < len { chars[idx] } else { '\0' };
    if idx + 1 < len {
        return Err(PyError::value_error(format!(
            "Invalid format specifier '{}' for object of type 'Fraction'",
            spec
        )));
    }
    if !matches!(ptype, 'e' | 'E' | 'f' | 'F' | 'g' | 'G' | '%') {
        return Err(PyError::value_error(format!(
            "Invalid format specifier '{}' for object of type 'Fraction'",
            spec
        )));
    }
    // Illegal to combine an explicit fill/alignment with zero padding
    // (CPython's "Refuse the temptation to guess" rule).
    if zeropad && align_explicit {
        return Err(PyError::value_error(format!(
            "Invalid format specifier '{}' for object of type 'Fraction'",
            spec
        )));
    }
    if align == '=' && fill == '0' {
        zeropad = true;
    }
    let pos_sign = if sign == '-' {
        String::new()
    } else {
        sign.to_string()
    };
    let trim_zeros = matches!(ptype, 'g' | 'G') && !alt;
    let trim_point = !alt;
    let exponent_indicator = if matches!(ptype, 'E' | 'F' | 'G') {
        "E"
    } else {
        "e"
    };

    let (negative, significand, exponent, point_pos, scientific): (bool, BigInt, i64, usize, bool) =
        if matches!(ptype, 'f' | 'F' | '%') {
            let mut exponent = -(precision as i64);
            if ptype == '%' {
                exponent -= 2;
            }
            let (neg, sig) = frac_round_to_exponent(num, den, exponent);
            (neg, sig, exponent, precision, false)
        } else {
            let figures = if matches!(ptype, 'g' | 'G') {
                precision.max(1)
            } else {
                precision + 1
            };
            let (neg, sig, exp) = frac_round_to_figures(num, den, figures);
            let scientific = matches!(ptype, 'e' | 'E') || exp > 0 || exp + figures as i64 <= -4;
            let point_pos = if scientific {
                figures - 1
            } else {
                (-exp) as usize
            };
            (neg, sig, exp, point_pos, scientific)
        };

    let suffix = if ptype == '%' {
        "%".to_string()
    } else if scientific {
        format!("{}{:+03}", exponent_indicator, exponent + point_pos as i64)
    } else {
        String::new()
    };

    let sig_str = significand.to_string();
    let negative = if no_neg_zero && significand.is_zero() {
        false
    } else {
        negative
    };
    let digits = format!("{:0>width$}", sig_str, width = point_pos + 1);
    let sign_out = if negative { "-" } else { &pos_sign };
    let leading = &digits[..digits.len() - point_pos];
    let mut frac_part = digits[digits.len() - point_pos..].to_string();
    if trim_zeros {
        frac_part = frac_part.trim_end_matches('0').to_string();
    }
    let separator = if trim_point && frac_part.is_empty() {
        ""
    } else {
        "."
    };
    let frac_part = if let Some(sep) = frac_sep {
        frac_part
            .chars()
            .collect::<Vec<char>>()
            .chunks(3)
            .map(|c| c.iter().collect::<String>())
            .collect::<Vec<_>>()
            .join(&sep.to_string())
    } else {
        frac_part
    };
    if separator.is_empty() && frac_part.is_empty() {
        // no-op
    }
    let mut leading = leading.to_string();
    if zeropad {
        // Zero-padding inserts '0's into the INTEGER DIGITS (before any
        // grouping) until sign + grouped digits + rest reaches the width.
        let rest_len = separator.len() + frac_part.len() + suffix.len();
        let sign_len = sign_out.len();
        let grouped_len = |d: usize| if d == 0 { 0 } else { d + (d - 1) / 3 };
        let d0 = leading.len();
        let mut d = d0;
        while sign_len + grouped_len(d) + rest_len < width {
            d += 1;
        }
        if d > d0 {
            leading = format!("{:0>width$}", leading, width = d);
        }
    }
    if let Some(sep) = int_sep {
        let mut g = String::new();
        let bytes: Vec<char> = leading.chars().collect();
        for (i, c) in bytes.iter().enumerate() {
            if i > 0 && (bytes.len() - i) % 3 == 0 {
                g.push(sep);
            }
            g.push(*c);
        }
        leading = g;
    }
    let body = format!(
        "{}{}{}{}{}",
        sign_out, leading, separator, frac_part, suffix
    );
    // Apply fill/align/width. Zero-padding (the '0' flag) pads with '0'
    // AFTER the sign, i.e. '=' alignment with fill '0'.
    if body.len() >= width {
        return Ok(body);
    }
    let pad = width - body.len();
    let eff_fill = if zeropad { '0' } else { fill };
    let eff_align = if zeropad && align != '<' && align != '^' {
        '='
    } else {
        align
    };
    if eff_align == '=' {
        let (prefix, rest) =
            if body.starts_with('-') || body.starts_with('+') || body.starts_with(' ') {
                body.split_at(1)
            } else {
                ("", body.as_str())
            };
        Ok(format!(
            "{}{}{}",
            prefix,
            eff_fill.to_string().repeat(pad),
            rest
        ))
    } else {
        match eff_align {
            '<' => Ok(format!("{}{}", body, eff_fill.to_string().repeat(pad))),
            '^' => {
                let left = pad / 2;
                let right = pad - left;
                Ok(format!(
                    "{}{}{}",
                    eff_fill.to_string().repeat(left),
                    body,
                    eff_fill.to_string().repeat(right)
                ))
            }
            _ => Ok(format!("{}{}", eff_fill.to_string().repeat(pad), body)),
        }
    }
}

/// Fallback that gets routed (by address, see vm.rs's call_function) to the
/// real `fraction_init_with_vm` — Fraction's constructor needs a live VM to
/// invoke user-provided `as_integer_ratio()` methods.
///
/// The address-based routing in call_function only fires for the *raw*
/// BuiltinFunction/BoundMethod objects; a bound copy produced through some
/// attribute-binding paths loses the original fn identity, so as a last
/// resort the fallback itself grabs the active VM via the VM_PTR
/// thread-local (always set while interpreter bytecode is running).
pub fn fraction_init_fallback(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    Ok(crate::object::with_vm_mut(|vm| fraction_init_with_vm(vm, args))??)
}

pub fn fraction_from_number_fallback(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    Ok(crate::object::with_vm_mut(|vm| fraction_from_number_with_vm(vm, args))??)
}

pub fn fraction_from_decimal_fallback(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    Ok(crate::object::with_vm_mut(|vm| fraction_from_decimal_with_vm(vm, args))??)
}

pub fn fraction_from_number_with_vm(
    vm: &mut crate::vm::VirtualMachine,
    args: &[PyObjectRef],
) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Err(PyError::type_error(
            "from_number() takes exactly 1 argument",
        ));
    }
    let cls = &args[0];
    let val = &args[1];
    let b = val.borrow();
    if matches!(&*b, PyObject::Str(_)) {
        return Err(PyError::type_error("argument should be a string or a Rational instance or have the as_integer_ratio() method"));
    }
    if matches!(&*b, PyObject::Complex(..)) {
        return Err(PyError::type_error("argument should be a string or a Rational instance or have the as_integer_ratio() method"));
    }
    drop(b);
    let (n, d) = frac_ctor_value(vm, val)?;
    frac_make(cls, n, d)
}

pub fn fraction_from_decimal_with_vm(
    vm: &mut crate::vm::VirtualMachine,
    args: &[PyObjectRef],
) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Err(PyError::type_error(
            "from_decimal() takes exactly 1 argument",
        ));
    }
    let cls = &args[0];
    let val = &args[1];
    let (n, d) = frac_ctor_value(vm, val)?;
    frac_make(cls, n, d)
}

/// Bind a raw class-dict value (as returned by `get_attribute`) to `obj`,
/// mirroring LOAD_ATTR's own auto-binding for instance method calls.
pub fn frac_bind_method(
    found: &PyObjectRef,
    obj: &PyObjectRef,
    typ: &PyObjectRef,
) -> Option<PyObjectRef> {
    let b = found.borrow();
    match &*b {
        PyObject::StaticMethod { func } => Some(func.clone()),
        PyObject::ClassMethod { func } => Some(PyObjectRef::imm(PyObject::BoundMethod {
            func: func.clone(),
            self_obj: typ.clone(),
        })),
        PyObject::Function(_) => Some(PyObjectRef::imm(PyObject::BoundMethod {
            func: found.clone(),
            self_obj: obj.clone(),
        })),
        PyObject::BuiltinFunction { name, func } => {
            Some(PyObjectRef::imm(PyObject::BuiltinMethod {
                name: name.clone(),
                func: *func,
                self_obj: obj.clone(),
            }))
        }
        PyObject::BuiltinMethod { name, func, .. } => {
            Some(PyObjectRef::imm(PyObject::BuiltinMethod {
                name: name.clone(),
                func: *func,
                self_obj: obj.clone(),
            }))
        }
        _ => None,
    }
}

/// One Rational operand for `Fraction(a, b)` / `Fraction(x)`: exact ints,
/// bools, Fraction instances, or any non-type object exposing
/// `as_integer_ratio()` (floats, Decimals, user Ratios, ...).
pub fn frac_ctor_value(
    vm: &mut crate::vm::VirtualMachine,
    obj: &PyObjectRef,
) -> PyResult<(BigInt, BigInt)> {
    if let PyObject::Int(n) = &*obj.borrow() {
        return Ok((n.clone(), BigInt::one()));
    }
    if let PyObject::Bool(b) = &*obj.borrow() {
        return Ok((BigInt::from(*b as i64), BigInt::one()));
    }
    if let Some((n, d)) = frac_instance_num_den(obj) {
        return Ok((n, d));
    }
    if let FracOperand::Float(f) = frac_operand_of(obj) {
        return frac_float_to_ratio(f);
    }
    if let PyObject::Str(s) = &*obj.borrow() {
        return frac_parse_str(s);
    }
    // The `numbers.Rational` protocol: read `.numerator` / `.denominator`
    // attributes (properties included) on arbitrary non-type objects —
    // checking the INSTANCE dict first (a `Rat`-style class stores them as
    // plain attributes), then the type dict/property resolution.
    if !matches!(&*obj.borrow(), PyObject::Type { .. }) {
        let (num, den) = if let PyObject::Instance { dict, .. } = &*obj.borrow() {
            let num = dict.get_str("numerator").cloned();
            let den = dict.get_str("denominator").cloned();
            if num.is_some() && den.is_some() {
                (num, den)
            } else {
                (
                    vm.resolve_descriptor_attr(obj, "numerator"),
                    vm.resolve_descriptor_attr(obj, "denominator"),
                )
            }
        } else {
            (
                vm.resolve_descriptor_attr(obj, "numerator"),
                vm.resolve_descriptor_attr(obj, "denominator"),
            )
        };
        if let (Some(num), Some(den)) = (num, den) {
            let n = crate::object::int_value_or_backing(&num)
                .or_else(|| crate::object::to_index(&num).ok())
                .or_else(|| {
                    num.as_f64().and_then(|f| {
                        if f.is_finite() {
                            Some(BigInt::from(f as i64))
                        } else {
                            None
                        }
                    })
                });
            let d = crate::object::int_value_or_backing(&den)
                .or_else(|| crate::object::to_index(&den).ok());
            if let (Some(n), Some(d)) = (n, d) {
                return Ok((n, d));
            }
        }
    }
    let is_type = matches!(&*obj.borrow(), PyObject::Type { .. });
    if !is_type {
        // An `as_integer_ratio` stored directly in the INSTANCE dict is
        // already bound (no `self` gets prepended on call) — a lambda like
        // `a.as_integer_ratio = lambda: (9, 5)`.
        let instance_attr = if let PyObject::Instance { dict, .. } = &*obj.borrow() {
            dict.get_str("as_integer_ratio").cloned()
        } else {
            None
        };
        let as_integer_ratio: Option<PyObjectRef> = if let Some(found) = instance_attr {
            Some(found)
        } else {
            let typ = if let PyObject::Instance { typ, .. } = &*obj.borrow() {
                Some(typ.clone())
            } else {
                None
            };
            if let (Some(typ), Ok(found)) = (typ, obj.borrow().get_attribute("as_integer_ratio")) {
                frac_bind_method(&found, obj, &typ)
            } else {
                None
            }
        };
        if let Some(bound) = as_integer_ratio {
            let result = vm.call_function(bound, vec![], vec![])?;
            let b = result.borrow();
            if let PyObject::Tuple(items) = &*b {
                if items.len() != 2 {
                    let msg = if items.len() < 2 {
                        format!(
                            "not enough values to unpack (expected 2, got {})",
                            items.len()
                        )
                    } else {
                        "too many values to unpack (expected 2)".to_string()
                    };
                    drop(b);
                    return Err(PyError::value_error(msg));
                }
                let num = crate::object::int_value_or_backing(&items[0])
                    .or_else(|| crate::object::to_index(&items[0]).ok())
                    .ok_or_else(|| {
                        PyError::type_error("as_integer_ratio() must return a pair of integers")
                    })?;
                let den = crate::object::int_value_or_backing(&items[1])
                    .or_else(|| crate::object::to_index(&items[1]).ok())
                    .ok_or_else(|| {
                        PyError::type_error("as_integer_ratio() must return a pair of integers")
                    })?;
                drop(b);
                return Ok((num, den));
            }
            drop(b);
            return Err(PyError::type_error(
                "cannot unpack non-iterable type from as_integer_ratio()",
            ));
        }
    }
    Err(PyError::type_error(
        "argument should be a string or a Rational instance or have the as_integer_ratio() method",
    ))
}

/// Raw numerator/denominator OBJECTS for a constructor operand — exact ints/
/// bools/Fractions/floats/strings become plain ints, but an int-subclass or
/// registered-Rational operand keeps its `.numerator`/`.denominator` objects
/// as-is (CPython stores these raw, so `F(myint(3), myint(6)).numerator` is
/// a `myint`).
pub fn frac_ctor_raw(
    vm: &mut crate::vm::VirtualMachine,
    obj: &PyObjectRef,
    allow_as_integer_ratio: bool,
    strict_rational: bool,
) -> PyResult<Option<(PyObjectRef, PyObjectRef)>> {
    if let PyObject::Int(n) = &*obj.borrow() {
        return Ok(Some((py_int(n.clone()), py_int(1))));
    }
    if let PyObject::Bool(b) = &*obj.borrow() {
        return Ok(Some((py_int(*b as i64), py_int(1))));
    }
    if let Some((n, d)) = frac_instance_num_den(obj) {
        return Ok(Some((py_int(n), py_int(d))));
    }
    if !strict_rational {
        if let FracOperand::Float(f) = frac_operand_of(obj) {
            let (n, d) = frac_float_to_ratio(f)?;
            return Ok(Some((py_int(n), py_int(d))));
        }
        if let PyObject::Str(s) = &*obj.borrow() {
            let (n, d) = frac_parse_str(s)?;
            return Ok(Some((py_int(n), py_int(d))));
        }
    }
    if !matches!(&*obj.borrow(), PyObject::Type { .. }) {
        let (num, den) = if let PyObject::Instance { dict, .. } = &*obj.borrow() {
            let num = dict.get_str("numerator").cloned();
            let den = dict.get_str("denominator").cloned();
            if num.is_some() && den.is_some() {
                (num, den)
            } else {
                (
                    vm.resolve_descriptor_attr(obj, "numerator"),
                    vm.resolve_descriptor_attr(obj, "denominator"),
                )
            }
        } else {
            (
                vm.resolve_descriptor_attr(obj, "numerator"),
                vm.resolve_descriptor_attr(obj, "denominator"),
            )
        };
        if let (Some(num), Some(den)) = (num, den) {
            return Ok(Some((num, den)));
        }
        // `as_integer_ratio` (instance-dict lambdas stay unbound) — only the
        // SINGLE-argument constructor form accepts these; the two-arg form
        // requires real Rational instances (CPython: `F(Ratio((3,7)), 11)`
        // raises TypeError).
        // raises TypeError).
        if allow_as_integer_ratio {
            let instance_attr = if let PyObject::Instance { dict, .. } = &*obj.borrow() {
                dict.get_str("as_integer_ratio").cloned()
            } else {
                None
            };
            let as_integer_ratio: Option<PyObjectRef> = if let Some(found) = instance_attr {
                Some(found)
            } else {
                let typ = if let PyObject::Instance { typ, .. } = &*obj.borrow() {
                    Some(typ.clone())
                } else {
                    None
                };
                if let (Some(typ), Ok(found)) =
                    (typ, obj.borrow().get_attribute("as_integer_ratio"))
                {
                    frac_bind_method(&found, obj, &typ)
                } else {
                    None
                }
            };
            if let Some(bound) = as_integer_ratio {
                let result = vm.call_function(bound, vec![], vec![])?;
                let b = result.borrow();
                if let PyObject::Tuple(items) = &*b {
                    if items.len() != 2 {
                        let msg = if items.len() < 2 {
                            format!(
                                "not enough values to unpack (expected 2, got {})",
                                items.len()
                            )
                        } else {
                            "too many values to unpack (expected 2)".to_string()
                        };
                        drop(b);
                        return Err(PyError::value_error(msg));
                    }
                    let num = crate::object::int_value_or_backing(&items[0])
                        .or_else(|| crate::object::to_index(&items[0]).ok())
                        .ok_or_else(|| {
                            PyError::type_error("as_integer_ratio() must return a pair of integers")
                        })?;
                    let den = crate::object::int_value_or_backing(&items[1])
                        .or_else(|| crate::object::to_index(&items[1]).ok())
                        .ok_or_else(|| {
                            PyError::type_error("as_integer_ratio() must return a pair of integers")
                        })?;
                    drop(b);
                    return Ok(Some((py_int(num), py_int(den))));
                }
                drop(b);
                return Err(PyError::type_error(
                    "cannot unpack non-iterable type from as_integer_ratio()",
                ));
            }
        }
    }
    Ok(None)
}

/// Normalize raw numerator/denominator objects to lowest terms with a
/// positive denominator, dividing the RAW objects by the gcd (CPython's
/// `numerator //= g; denominator //= g` on the stored objects).
pub fn frac_normalize_raw(
    num: &PyObjectRef,
    den: &PyObjectRef,
) -> PyResult<(PyObjectRef, PyObjectRef)> {
    let ni = crate::object::int_value_or_backing(num).or_else(|| crate::object::to_index(num).ok());
    let di = crate::object::int_value_or_backing(den).or_else(|| crate::object::to_index(den).ok());
    let (ni, di) = match (ni, di) {
        (Some(n), Some(d)) => (n, d),
        _ => return Ok((num.clone(), den.clone())),
    };
    if di.is_zero() {
        return Err(PyError::ZeroDivisionError(format!("Fraction({}, 0)", ni)));
    }
    let g_pos = frac_bigint_gcd(&ni, &di);
    let g = if di.sign() == num_bigint::Sign::Minus {
        -g_pos
    } else {
        g_pos
    };
    // CPython divides ALWAYS (`numerator //= g`), which is a no-op for g == 1
    // but flips the sign for a negative g.
    let num = crate::object::py_floor_div(num, &py_int(g.clone()))?;
    let den = crate::object::py_floor_div(den, &py_int(g))?;
    Ok((num, den))
}

/// Fraction's real constructor (CPython's `Fraction.__new__`): single-arg
/// int / Rational / float / string / as_integer_ratio object, or the
/// two-arg numerator/denominator (each an int or Rational) form.
pub fn fraction_init_with_vm(
    vm: &mut crate::vm::VirtualMachine,
    args: &[PyObjectRef],
) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error("__init__ requires self"));
    }
    let rest = &args[1..];
    let (num, den): (PyObjectRef, PyObjectRef) = match rest.len() {
        0 => (py_int(0), py_int(1)),
        1 => frac_ctor_raw(vm, &rest[0], true, false)?.ok_or_else(|| {
            PyError::type_error(
                "argument should be a string or a Rational instance or have the as_integer_ratio() method",
            )
        })?,
        2 => {
            let (an, ad) = frac_ctor_raw(vm, &rest[0], false, true)?.ok_or_else(|| {
                PyError::type_error("both arguments should be Rational instances")
            })?;
            let (bn, bd) = frac_ctor_raw(vm, &rest[1], false, true)?.ok_or_else(|| {
                PyError::type_error("both arguments should be Rational instances")
            })?;
            let num = crate::object::py_mul(&an, &bd)?;
            let den = crate::object::py_mul(&ad, &bn)?;
            frac_normalize_raw(&num, &den)?
        }
        _ => return Err(PyError::type_error("Fraction() takes at most 2 arguments")),
    };
    if let PyObject::Instance { dict, .. } = &mut *args[0].borrow_mut() {
        // Immutable: a re-run `r.__init__(...)` on an already-built
        // Fraction is a no-op (CPython's slots-based Fraction).
        if dict.get_str("_numerator").is_some() {
            return Ok(py_none());
        }
        dict.insert_str("_numerator", num);
        dict.insert_str("_denominator", den);
    }
    Ok(py_none())
}

