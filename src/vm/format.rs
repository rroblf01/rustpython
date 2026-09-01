use crate::bytecode::*;
use crate::interner::{self, InternedMap, StrId};
use crate::object::*;
use num_bigint::BigInt;
use num_traits::ToPrimitive;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use smallvec::SmallVec;

/// Implements Python's Format Specification Mini-Language.
///
/// Parses a format spec string in the form:
/// `[[fill]align][sign][#][0][width][grouping_option][.precision][type]`
/// and applies the formatting to the given value.
///
/// See: https://docs.python.org/3/library/string.html#formatspec
pub fn format_with_spec(val: &PyObjectRef, spec_str: &str) -> PyResult<String> {
    // An Instance with a custom `__format__` (Fraction, Decimal, a user
    // class) formats via its dunder — format(Fraction(1,3), '.2f') must
    // reach Fraction.__format__, not fall through to str().
    let instance_format = {
        let v = val.borrow();
        if let PyObject::Instance { typ, .. } = &*v {
            lookup_dunder_via_mro(typ, "__format__")
        } else {
            None
        }
    };
    if let Some(f) = instance_format {
        let result = call_bound_method(f, val.clone(), vec![py_str(spec_str)])?;
        return Ok(result.str());
    }
    if spec_str.is_empty() {
        return Ok(val.str());
    }

    let chars: Vec<char> = spec_str.chars().collect();
    let len = chars.len();
    let mut idx = 0;

    // --- parse [[fill]align] ---
    let fill_char;
    let align;
    let align_explicit;
    if idx + 1 < len && matches!(chars[idx + 1], '<' | '>' | '^' | '=') {
        fill_char = chars[idx];
        align = chars[idx + 1];
        align_explicit = true;
        idx += 2;
    } else if idx < len && matches!(chars[idx], '<' | '>' | '^' | '=') {
        fill_char = ' ';
        align = chars[idx];
        align_explicit = true;
        idx += 1;
    } else {
        fill_char = ' ';
        align = '>';
        align_explicit = false;
    }

    // --- parse [sign] ---
    let sign = if idx < len && matches!(chars[idx], '+' | '-' | ' ') {
        let s = chars[idx];
        idx += 1;
        s
    } else {
        '-' // default: show sign only for negatives
    };

    // --- parse [z] (3.11+ float zero-coercion flag) ---
    // Forces +0.0/-0.0 to plain 0.0. Parsed here so the spec isn't rejected
    // as invalid (the coercion itself is applied in the float arms by
    // zeroing negative zero).
    let zero_coerce = if idx < len && chars[idx] == 'z' {
        idx += 1;
        true
    } else {
        false
    };
    // --- parse [#] ---
    let alternate = if idx < len && chars[idx] == '#' {
        idx += 1;
        true
    } else {
        false
    };

    // --- parse [0] (zero-pad flag) ---
    // Note: '0' after width means just a digit, not zero-pad.
    // But Python's spec has '0' right after the sign/# before width.
    // We check if the next char is '0' AND is followed by a digit (width).
    let mut zero_pad = false;
    if idx < len && chars[idx] == '0' {
        // If '0' is followed by a digit or end, it's the start of width with zero-padding
        zero_pad = true;
        if idx + 1 < len && chars[idx + 1].is_ascii_digit() {
            idx += 1; // consume the '0' — it becomes part of width
        } else {
            idx += 1; // just '0' with no width
        }
    }

    // --- parse [width] ---
    let width: Option<usize> = {
        let start = idx;
        while idx < len && chars[idx].is_ascii_digit() {
            idx += 1;
        }
        if idx > start {
            // A format spec width this large (more digits than fit in a
            // `usize`) is nonsensical for any real display — real CPython
            // raises `ValueError` for absurd widths/precisions (deliberately
            // tested: CPython's own `test_format.py::test_precision` builds
            // a `.%sf % (sys.maxsize + 1)` spec specifically to check this)
            // rather than crashing. Bare `.unwrap()` here panicked the whole
            // process on `ParseIntError` instead.
            let w = chars[start..idx]
                .iter()
                .collect::<String>()
                .parse::<usize>()
                .map_err(|_| PyError::value_error("Format specifier width too large"))?;
            // Unlike the overflow case above, a width of e.g. `sys.maxsize +
            // 1` (2**63) parses into a `usize` just fine — but actually
            // padding a string out to that length tries to allocate an
            // astronomical buffer (`apply_padding`'s `fill.repeat(w -
            // s.len())`), aborting the whole process with "memory
            // allocation of N bytes failed" instead of raising a catchable
            // `ValueError`. Same real trigger as the precision cap below
            // (`test_format.py::test_format_huge_width`, `.../huge_item_
            // number`) — capped at the same threshold for consistency.
            if w > 1000 {
                return Err(PyError::value_error("Format specifier width too large"));
            }
            Some(w)
        } else {
            None
        }
    };

    // Go back if we consumed '0' but it wasn't really zero-pad (no width follows)
    if zero_pad && width.is_none() {
        // The '0' was just a literal zero in a width-less spec — not valid, treat as no-op
        zero_pad = false;
    }

    // --- parse grouping option [,|_] (integer part) ---
    let int_grouping: Option<char> = if idx < len && (chars[idx] == ',' || chars[idx] == '_') {
        let g = chars[idx];
        idx += 1;
        Some(g)
    } else {
        None
    };
    // A second grouping char right after the first is a repeat/mix error
    // ('{:,,}', '{:__}', '{:,_}', '{:_,}').
    if int_grouping.is_some() && idx < len && (chars[idx] == ',' || chars[idx] == '_') {
        let g = chars[idx];
        if int_grouping == Some(g) {
            return Err(PyError::value_error(format!(
                "Cannot specify '{}' with '{}'.",
                g, g
            )));
        }
        return Err(PyError::value_error("Cannot specify both ',' and '_'."));
    }

    // --- parse [.precision] and fraction-part grouping ---
    // CPython splits the grouping option into TWO: one before the precision
    // (integer part) and one after (fraction part) — format(x, '._f') groups
    // the digits after the point. A '.' with no digits AND no trailing
    // grouping char is "Format specifier missing precision".
    let (precision, frac_grouping): (Option<usize>, Option<char>) =
        if idx < len && chars[idx] == '.' {
            idx += 1;
            let start = idx;
            while idx < len && chars[idx].is_ascii_digit() {
                idx += 1;
            }
            let p = if idx > start {
                // See the matching `width` comment above — same overflow-panic
                // fix, same real trigger (`test_format.py::test_precision`'s
                // `.%sf % (sys.maxsize + 1)`).
                let p = chars[start..idx]
                    .iter()
                    .collect::<String>()
                    .parse::<usize>()
                    .map_err(|_| PyError::value_error("Format specifier precision too large"))?;
                // A precision this large parses fine as a `usize` (e.g.
                // `sys.maxsize + 1` == 2**63, well within range) but Rust's own
                // `format!("{:.prec$}", ...)` panics with "argument out of
                // range" trying to render it (asking for ~9*10^18 decimal
                // digits of a float is obviously never actually intended) —
                // confirmed via CPython's own `test_format.py::test_precision`,
                // which deliberately builds `.%sf % (sys.maxsize + 1)`
                // expecting a catchable `ValueError`, not a process crash.
                // 1000 decimal digits is already far beyond any real
                // formatting need (a `f64`'s own precision exhausts after ~17
                // significant digits) but comfortably below wherever Rust's
                // internal limit actually sits.
                if p > 1000 {
                    return Err(PyError::value_error("precision too big"));
                }
                Some(p)
            } else {
                None
            };
            // Fraction-part grouping directly after the '.'/precision digits.
            let fg: Option<char> = if idx < len && (chars[idx] == ',' || chars[idx] == '_') {
                let g = chars[idx];
                idx += 1;
                Some(g)
            } else {
                None
            };
            if p.is_none() && fg.is_none() {
                return Err(PyError::value_error("Format specifier missing precision"));
            }
            // Repeat/mix check in the fraction position, and mixing the fraction
            // separator with a DIFFERENT integer-part separator ('{:.,_f}').
            if fg.is_some() && idx < len && (chars[idx] == ',' || chars[idx] == '_') {
                if fg == Some(chars[idx]) {
                    return Err(PyError::value_error(format!(
                        "Cannot specify '{}' with '{}'.",
                        chars[idx], chars[idx]
                    )));
                }
                return Err(PyError::value_error("Cannot specify both ',' and '_'."));
            }
            if int_grouping.is_some() && fg.is_some() && int_grouping != fg {
                return Err(PyError::value_error("Cannot specify both ',' and '_'."));
            }
            (p, fg)
        } else {
            (None, None)
        };

    // --- parse [type] ---
    let fmt_type = if idx < len {
        let t = chars[idx];
        idx += 1;
        Some(t)
    } else {
        None
    };

    // Digit grouping width: 4 for hex/oct/bin, 3 for everything else.
    // CPython groups hex/oct/bin by 4 digits with '_' (e.g. '0x1234_5678').
    let int_group_width = match fmt_type {
        Some('x' | 'X' | 'b' | 'o') => 4,
        _ => 3,
    };

    // Any characters left unconsumed mean an invalid specifier — real
    // CPython raises `ValueError: Invalid format specifier '<spec>' for
    // object of type '<type>'` (test_format's
    // test_better_error_message_format asserts the exact message). The
    // type char itself was consumed above, so a trailing second type char
    // (e.g. '%M' -> '%' consumed as type, 'M' leftover) lands here.
    if idx < len {
        let typename = val.borrow().type_name().to_string();
        return Err(PyError::value_error(format!(
            "Invalid format specifier '{}' for object of type '{}'",
            spec_str, typename
        )));
    }

    // `z` is only allowed for float values with a float presentation type —
    // `{0:zd}`, `{0:z}` (int) and `{'x':zs}` (str) all raise
    // "Negative zero coercion (z) not allowed".
    if zero_coerce {
        let t = fmt_type.unwrap_or('\0');
        // z is allowed with a FLOAT presentation (f/e/g/%) for any numeric
        // value (an int with z.1f is fine), and with the DEFAULT type only
        // for an actual float. Non-float presentations (d/s/x...) reject it.
        let is_float_present =
            t == 'f' || t == 'F' || t == 'e' || t == 'E' || t == 'g' || t == 'G' || t == '%';
        let is_default_float = t == '\0' && matches!(&*val.borrow(), PyObject::Float(_));
        if !(is_float_present || is_default_float) {
            return Err(PyError::value_error(
                "Negative zero coercion (z) not allowed",
            ));
        }
    }

    // Grouping (','/'_') is only valid with compatible presentation types
    // ('{:,.3s}' / '{:,,.3f}' raise "Cannot specify ',' with 's'."; '_' is
    // additionally allowed for b/o/x/X where it groups every four digits).
    if let Some(sep) = int_grouping {
        let t = fmt_type.unwrap_or('\0');
        let allowed = matches!(t, 'd' | 'e' | 'f' | 'g' | 'E' | 'G' | '%' | 'F' | '\0')
            || (sep == '_' && matches!(t, 'b' | 'o' | 'x' | 'X'));
        if !allowed {
            if t > '\u{20}' && t < '\u{80}' {
                return Err(PyError::value_error(format!(
                    "Cannot specify '{}' with '{}'.",
                    sep, t
                )));
            }
            return Err(PyError::value_error(format!(
                "Cannot specify '{}' with '\\x{:x}'.",
                sep, t as u32
            )));
        }
    }
    // The fraction-part grouping is incompatible with the 'n' type
    // ('{:,.3n}' raises "Cannot specify ',' with 'n'.").
    if let Some(sep) = frac_grouping {
        if fmt_type == Some('n') {
            return Err(PyError::value_error(format!(
                "Cannot specify '{}' with 'n'.",
                sep
            )));
        }
    }

    // Determine value type
    let val_borrowed = val.borrow();
    let is_int = matches!(&*val_borrowed, PyObject::Int(_) | PyObject::Bool(_));
    let is_float = matches!(&*val_borrowed, PyObject::Float(_));

    // A float only accepts float presentation types — 's'/'d'/'o'/'x'/'X'/
    // 'b'/'c' raise ValueError (format(3.0, 's') is an error, not str()).
    if is_float {
        if let Some(t) = fmt_type {
            if !matches!(t, 'e' | 'E' | 'f' | 'F' | 'g' | 'G' | 'n' | '%') {
                return Err(PyError::value_error(format!(
                    "Unknown format code '{}' for object of type 'float'",
                    t
                )));
            }
        }
    }

    // Complex formatting: apply the spec to BOTH parts and join
    // (format(1.2+0j, '.0f') == '1+0j').
    let complex_parts = if let PyObject::Complex(re, im) = &*val_borrowed {
        Some((*re, *im))
    } else {
        None
    };
    if let Some((re, im)) = complex_parts {
        let spec_clone = spec_str.to_string();
        // A spec with a numeric TYPE (f/e/g/%/n) applies to EACH part
        // (format(1+2j, '.1f') == '1.0+2.0j'); width/align-only specs apply
        // to the whole complex string (format(1+2j, '<8') == '(1+2j)  ').
        let has_numeric_type = spec_str
            .trim_end_matches(|c: char| c.is_ascii_digit())
            .chars()
            .last()
            .map(|c| matches!(c, 'f' | 'F' | 'e' | 'E' | 'g' | 'G' | '%' | 'n'))
            .unwrap_or(false);
        if !has_numeric_type {
            drop(val_borrowed);
            // Format the complex via str(), then apply the spec as a string.
            let s = val.str();
            let p = py_str(&s);
            return crate::vm::format::format_with_spec(&p, &spec_clone);
        }
        let fmt_part = |part: f64| -> PyResult<String> {
            let p = py_float(part);
            crate::vm::format::format_with_spec(&p, &spec_clone)
        };
        let re_s = fmt_part(re)?;
        // Determine the imag sign from the FORMATTED part (z already coerced
        // a rounded -0.0 to positive, and a negative mantissa keeps its -).
        let im_abs = fmt_part(im.abs())?;
        let im_signed = fmt_part(im)?;
        let sign = if im_signed.starts_with('-') { "-" } else { "+" };
        return Ok(format!("{}{}{}j", re_s, sign, im_abs));
    }

    // Generate the formatted value based on type
    let base = match (fmt_type, is_int, is_float) {
        // Integer: decimal (default or 'd')
        (None, true, _) | (Some('d'), true, _) => {
            if let PyObject::Int(i) = &*val_borrowed {
                let s = format_int_with_sign(i, sign, precision);
                s
            } else if let PyObject::Bool(b) = &*val_borrowed {
                format!("{}", if *b { 1i64 } else { 0i64 })
            } else {
                val.str()
            }
        }
        // Integer: hex lowercase
        (Some('x'), true, _) => {
            if let PyObject::Int(i) = &*val_borrowed {
                let digits = format!("{:x}", i.magnitude());
                let prefix = if alternate { "0x" } else { "" };
                let sign_str = if i.sign() == num_bigint::Sign::Minus {
                    "-".to_string()
                } else {
                    match sign {
                        '+' => "+".to_string(),
                        ' ' => " ".to_string(),
                        _ => String::new(),
                    }
                };
                format!("{}{}{}", sign_str, prefix, digits)
            } else {
                val.str()
            }
        }
        // Integer: hex uppercase
        (Some('X'), true, _) => {
            if let PyObject::Int(i) = &*val_borrowed {
                let digits = format!("{:X}", i.magnitude());
                let prefix = if alternate { "0X" } else { "" };
                let sign_str = if i.sign() == num_bigint::Sign::Minus {
                    "-".to_string()
                } else {
                    match sign {
                        '+' => "+".to_string(),
                        ' ' => " ".to_string(),
                        _ => String::new(),
                    }
                };
                format!("{}{}{}", sign_str, prefix, digits)
            } else {
                val.str()
            }
        }
        // Integer: binary
        (Some('b'), true, _) => {
            if let PyObject::Int(i) = &*val_borrowed {
                let digits = format!("{:b}", i.magnitude());
                let prefix = if alternate { "0b" } else { "" };
                let sign_str = if i.sign() == num_bigint::Sign::Minus {
                    "-".to_string()
                } else {
                    match sign {
                        '+' => "+".to_string(),
                        ' ' => " ".to_string(),
                        _ => String::new(),
                    }
                };
                format!("{}{}{}", sign_str, prefix, digits)
            } else {
                val.str()
            }
        }
        // Integer: octal
        (Some('o'), true, _) => {
            if let PyObject::Int(i) = &*val_borrowed {
                let digits = format!("{:o}", i.magnitude());
                let prefix = if alternate { "0o" } else { "" };
                let sign_str = if i.sign() == num_bigint::Sign::Minus {
                    "-".to_string()
                } else {
                    match sign {
                        '+' => "+".to_string(),
                        ' ' => " ".to_string(),
                        _ => String::new(),
                    }
                };
                format!("{}{}{}", sign_str, prefix, digits)
            } else {
                val.str()
            }
        }
        // Integer: character
        (Some('c'), true, _) => {
            if let PyObject::Int(i) = &*val_borrowed {
                if let Some(n) = i.to_u32() {
                    if let Some(c) = char::from_u32(n) {
                        c.to_string()
                    } else {
                        return Err(PyError::value_error("chr() arg not in range(0x110000)"));
                    }
                } else {
                    return Err(PyError::value_error("chr() arg not in range(0x110000)"));
                }
            } else {
                return Err(PyError::type_error("integer argument expected, got float"));
            }
        }

        // Float: default (no type) — no precision: str(); with precision it
        // behaves like 'g' (format(123.456, '.4') == '123.5').
        (None, _, true) => {
            if let Some(mut f) = val.as_f64() {
                if zero_coerce && f == 0.0 {
                    f = 0.0;
                }
                match precision {
                    Some(p) => {
                        let s = crate::object::format_percent_g(f, p, false, true);
                        apply_sign(&s, f, sign)
                    }
                    None => val.str(),
                }
            } else {
                val.str()
            }
        }
        // Float: fixed-point (an int with 'f' converts to float first)
        (Some('f'), _, _) | (Some('F'), _, _) => {
            if let Some(mut f) = val.as_f64() {
                if zero_coerce && f == 0.0 {
                    f = 0.0;
                }
                // 'f' defaults to precision 6 (like %f), not str() — real
                // CPython: format(0.0, 'f') == '0.000000'.
                let mut s = format_float_with_sign(f, sign, Some(precision.unwrap_or(6)));
                // 'F' upper-cases inf/nan (INF/NAN), like %F.
                if fmt_type == Some('F') && (f.is_nan() || f.is_infinite()) {
                    s = s.to_uppercase();
                }
                // the # flag keeps a trailing point for integral values, but
                // never for inf/nan (format(inf, '#f') == 'inf').
                if alternate && !s.contains('.') && !f.is_nan() && !f.is_infinite() {
                    s.push('.');
                }
                s
            } else {
                val.str()
            }
        }
        // Float: scientific lowercase
        (Some('e'), _, _) => {
            if let Some(mut f) = val.as_f64() {
                if zero_coerce && f == 0.0 {
                    f = 0.0;
                }
                let s =
                    crate::object::format_percent_e(f, precision.unwrap_or(6), alternate, false);
                apply_sign(&s, f, sign)
            } else {
                val.str()
            }
        }
        // Float: scientific uppercase
        (Some('E'), _, _) => {
            if let Some(mut f) = val.as_f64() {
                if zero_coerce && f == 0.0 {
                    f = 0.0;
                }
                let s = crate::object::format_percent_e(f, precision.unwrap_or(6), alternate, true);
                apply_sign(&s, f, sign)
            } else {
                val.str()
            }
        }
        // Float: general lowercase
        (Some('g'), _, _) => {
            if let Some(mut f) = val.as_f64() {
                if zero_coerce && f == 0.0 {
                    f = 0.0;
                }
                let s =
                    crate::object::format_percent_g(f, precision.unwrap_or(6), alternate, false);
                apply_sign(&s, f, sign)
            } else {
                val.str()
            }
        }
        // Float: general uppercase
        (Some('G'), _, _) => {
            if let Some(mut f) = val.as_f64() {
                if zero_coerce && f == 0.0 {
                    f = 0.0;
                }
                let s =
                    crate::object::format_percent_g(f, precision.unwrap_or(6), alternate, false)
                        .to_uppercase();
                apply_sign(&s, f, sign)
            } else {
                val.str()
            }
        }
        // Float: percentage
        (Some('%'), _, true) => {
            if let PyObject::Float(f) = &*val_borrowed {
                let pct = f * 100.0;
                // '%' defaults to 6 decimals like 'f' (format(-1.0, '%') ==
                // '-100.000000%'), not str()-style shortest repr.
                let s = format_float_with_sign(pct, sign, Some(precision.unwrap_or(6)));
                format!("{}%", s)
            } else {
                val.str()
            }
        }

        // Default for string or any other type: str() representation
        _ => val.str(),
    };

    // Zero-padding that merges zeros INTO the integer digits ('0' flag with
    // NO explicit alignment) must group the padded zeros too — CPython
    // format(x, '021_._f') == '0_000_123_456.123_456', not
    // '000000123_456.123_456'. Insert zeros one at a time until the grouped
    // length reaches the width (grouping can add a separator, so a width
    // like 020 can legitimately render as 21 chars).
    let zero_pad_group = |base: &str, width: usize| -> String {
        let mut s = base.to_string();
        loop {
            let grouped = apply_grouping(&s, int_grouping, frac_grouping, int_group_width);
            if grouped.len() >= width {
                return grouped;
            }
            let (sign, rest) = if let Some(r) = s.strip_prefix('-') {
                ("-", r)
            } else if let Some(r) = s.strip_prefix('+') {
                ("+", r)
            } else if let Some(r) = s.strip_prefix(' ') {
                (" ", r)
            } else {
                ("", s.as_str())
            };
            s = format!("{}0{}", sign, rest);
        }
    };

    // Thousands grouping (',' or '_'): insert the separator every 3 digits
    // of the integer part (format(1234567, ',') == '1,234,567') and/or the
    // fraction part (format(x, '._f') groups the digits after the point).
    let base = if zero_pad && !align_explicit && (int_grouping.is_some() || frac_grouping.is_some())
    {
        zero_pad_group(&base, width.unwrap_or(0))
    } else {
        apply_grouping(&base, int_grouping, frac_grouping, int_group_width)
    };

    // The `z` flag coerces a NEGATIVE ZERO RESULT (after rounding) to
    // positive zero — `f'{-.01:z.1f}'` rounds -0.01 to '-0.0' then coerces
    // to '0.0'. Applied to the formatted base before padding.
    let base = if zero_coerce && base.starts_with('-') {
        let rest = &base[1..];
        let is_zero_form = rest.parse::<f64>().map(|v| v == 0.0).unwrap_or(false);
        if is_zero_form {
            base[1..].to_string()
        } else {
            base
        }
    } else {
        base
    };

    // Apply zero-padding (fill='0'): with an explicit alignment the fill
    // chars are padding, not digits (format(x, '>021_._f') ==
    // '000000123_456.123_456'); the bare '0' flag already merged+grouped
    // its zeros above.
    let base = if zero_pad && align_explicit {
        apply_padding(&base, width, align, '0', false)
    } else {
        base
    };

    // Apply final width and alignment.
    // The '0' flag WITHOUT explicit alignment implies '=' alignment with fill='0'
    // for numeric types. '=' alignment inserts padding after sign AND after
    // any alternate-form prefix (0x, 0b, 0o).
    let (effective_align, effective_fill) = if zero_pad && !align_explicit {
        ('=', '0')
    } else {
        (align, fill_char)
    };
    let result = apply_padding(&base, width, effective_align, effective_fill, false);

    Ok(result)
}

/// Apply '+'/' '/'-' sign prefix. If `sign` is '-', only negative numbers get a '-'.
/// If `sign` is '+', positive numbers get '+', negative get '-'.
/// If `sign` is ' ', positive numbers get ' ', negative get '-'.
fn apply_sign(s: &str, val: f64, sign: char) -> String {
    if val < 0.0 {
        // Negative — Rust format already includes '-'
        format!("-{}", &s.trim_start_matches('-'))
    } else {
        match sign {
            '+' => format!("+{}", s),
            ' ' => format!(" {}", s),
            '-' => s.to_string(),
            _ => s.to_string(),
        }
    }
}

/// Format a BigInt with sign handling for Python format spec.
fn format_int_with_sign(i: &BigInt, sign: char, precision: Option<usize>) -> String {
    let s = if i.sign() == num_bigint::Sign::Minus {
        // Remove negative sign from BigInt's display, we'll add it back
        let abs_s = format!("{}", i).trim_start_matches('-').to_string();
        let s = match precision {
            Some(p) if p > abs_s.len() => {
                let zeros = "0".repeat(p - abs_s.len());
                format!("{}{}", zeros, abs_s)
            }
            _ => abs_s,
        };
        format!("-{}", s)
    } else {
        let abs_s = format!("{}", i);
        let s = match precision {
            Some(p) if p > abs_s.len() => {
                let zeros = "0".repeat(p - abs_s.len());
                format!("{}{}", zeros, abs_s)
            }
            _ => abs_s,
        };
        match sign {
            '+' => format!("+{}", s),
            ' ' => format!(" {}", s),
            '-' => s,
            _ => s,
        }
    };
    s
}

/// Format a float with sign and precision.
fn format_float_with_sign(val: f64, sign: char, precision: Option<usize>) -> String {
    if val.is_nan() {
        // CPython prints 'nan' (always signless by value; the +/space flags
        // still apply: format(nan, '+f') == '+nan').
        return apply_sign("nan", val, sign);
    }
    if val.is_infinite() {
        let s = if val.is_sign_negative() {
            "-inf"
        } else {
            "inf"
        }
        .to_string();
        return apply_sign(&s, val, sign);
    }
    let s = match precision {
        Some(p) => format!("{:.prec$}", val, prec = p),
        None => format!("{}", val),
    };
    apply_sign(&s, val, sign)
}

/// Insert ',' or '_' every N digits of the integer part (from the right) and
/// or of the fraction part (from the left) of a formatted number. For decimal
/// types ('d', 'n', etc.) N=3; for hex/oct/bin ('x', 'X', 'b', 'o') N=4.
/// Handles scientific suffixes (`e+05`) and the `%` suffix so the
/// mantissa/percent fraction can be grouped too.
fn apply_grouping(s: &str, int_sep: Option<char>, frac_sep: Option<char>, int_group_width: usize) -> String {
    if int_sep.is_none() && frac_sep.is_none() {
        return s.to_string();
    }
    let (sign, rest) = if let Some(r) = s.strip_prefix('-') {
        ("-", r)
    } else if let Some(r) = s.strip_prefix('+') {
        ("+", r)
    } else if let Some(r) = s.strip_prefix(' ') {
        (" ", r)
    } else {
        ("", s)
    };
    // Split off any exponent/percent suffix so it isn't grouped.
    let (body, suffix) = match rest.find(['e', 'E', '%']) {
        Some(p) => (&rest[..p], &rest[p..]),
        None => (rest, ""),
    };
    let (int_part, frac_part) = match body.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (body, None),
    };
    let group_int = |t: &str| -> String {
        match int_sep {
            Some(sep) => {
                let mut g = String::new();
                for (i, c) in t.chars().enumerate() {
                    if i > 0 && (t.len() - i) % int_group_width == 0 {
                        g.push(sep);
                    }
                    g.push(c);
                }
                g
            }
            None => t.to_string(),
        }
    };
    let group_frac = |t: &str| -> String {
        match frac_sep {
            Some(sep) => {
                let mut g = String::new();
                for (i, c) in t.chars().enumerate() {
                    if i > 0 && i % 3 == 0 {
                        g.push(sep);
                    }
                    g.push(c);
                }
                g
            }
            None => t.to_string(),
        }
    };
    let grouped_int = group_int(int_part);
    match frac_part {
        Some(f) => format!("{}{}.{}{}", sign, grouped_int, group_frac(f), suffix),
        None => format!("{}{}{}", sign, grouped_int, suffix),
    }
}

/// Apply padding/alignment to a base string.
fn apply_padding(
    s: &str,
    width: Option<usize>,
    align: char,
    fill: char,
    zero_mode: bool,
) -> String {
    let w = match width {
        Some(w) => w,
        None => return s.to_string(),
    };
    if s.len() >= w {
        return s.to_string();
    }
    let padding = w - s.len();
    let pad_str: String = fill.to_string().repeat(padding);

    match align {
        '<' => format!("{}{}", s, pad_str),
        '>' => format!("{}{}", pad_str, s),
        '^' => {
            let left = padding / 2;
            let right = padding - left;
            format!(
                "{}{}{}",
                fill.to_string().repeat(left),
                s,
                fill.to_string().repeat(right)
            )
        }
        '=' => {
            // Insert padding after sign (if any) and after any alternate-form
            // prefix (0x, 0X, 0b, 0o) but before digits.
            if zero_mode {
                format!("{}{}", pad_str, s)
            } else {
                let mut skip = 0;
                let bytes = s.as_bytes();
                if !bytes.is_empty() && matches!(bytes[0], b'+' | b'-' | b' ') {
                    skip = 1;
                }
                if skip < bytes.len() && bytes[skip] == b'0'
                    && skip + 1 < bytes.len()
                    && matches!(bytes[skip + 1], b'x' | b'X' | b'b' | b'o')
                {
                    skip += 2;
                }
                if skip > 0 {
                    let (prefix, rest) = s.split_at(skip);
                    format!("{}{}{}", prefix, pad_str, rest)
                } else {
                    format!("{}{}", pad_str, s)
                }
            }
        }
        _ => format!("{}{}", pad_str, s), // default right-align
    }
}
