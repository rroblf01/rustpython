// Extracted from pyobject.rs — float/complex formatting helpers.
// Kept small to stay <1k.
// Provides format_py_float, format_complex_part, escape_string.
use super::*;

pub(crate) fn format_py_float(f: f64) -> String {
    if f.is_nan() {
        "nan".to_string()
    } else if f.is_infinite() && f.is_sign_positive() {
        "inf".to_string()
    } else if f.is_infinite() {
        "-inf".to_string()
    } else {
        // Rust's `{:?}` on f64 is the SHORTEST round-trip representation
        // (Ryu) — the same unique digit string CPython's repr uses, where the
        // old `{:.17}` form always emitted 17 significant digits (1.3 became
        // "1.30000000000000004") and never used exponents (1e300 printed as a
        // giant integer). Only the EXPONENT syntax differs from Python:
        // Python always writes a sign for positive exponents and pads the
        // exponent to at least two digits (`1e-05`, `1e+16`).
        let mut s = format!("{:?}", f);
        if let Some(epos) = s.find('e') {
            let sign_pos = epos + 1;
            let has_sign = s[sign_pos..].starts_with('-') || s[sign_pos..].starts_with('+');
            if !has_sign {
                s.insert(sign_pos, '+');
            }
            // pad exponent to at least 2 digits: 1e-5 -> 1e-05
            let digits_start = sign_pos + 1;
            if s.len() - digits_start < 2 {
                s.insert(digits_start, '0');
            }
        }
        s
    }
}

pub(crate) fn format_complex_part(f: f64) -> String {
    if f.is_nan() {
        "nan".to_string()
    } else if f.is_infinite() && f.is_sign_positive() {
        "inf".to_string()
    } else if f.is_infinite() {
        "-inf".to_string()
    } else {
        let s = format_py_float(f);
        // A whole-number part loses its ".0" (`repr(2j)` == "2j", not
        // "2.0j") — Rust's shortest repr emits "2.0" for 2.0.
        if s.ends_with(".0") && !s.contains('e') {
            s[..s.len() - 2].to_string()
        } else {
            s
        }
    }
}

pub(crate) fn escape_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        // Escape non-ASCII chars that are NOT printable (CPython's repr
        // keeps printable non-ASCII like 'café'/U+0374 but escapes
        // unassigned/format chars like U+0378 as \\u0378). Approximation of
        // Unicode printability without a full DB: letters/digits/marks plus
        // common punctuation/symbol/space ranges are kept; everything else
        // is escaped.
        fn is_printable(c: char) -> bool {
            // All ASCII printable (space..~) are printable.
            if c.is_ascii() {
                return c >= ' ' && c != '\x7f';
            }
            if c.is_alphanumeric() || c.is_whitespace() {
                return true;
            }
            let cp = c as u32;
            // Common punctuation/symbol/space ranges (a coarse superset).
            matches!(cp,
                0x00A0..=0x00FF      // Latin-1 supplement (incl. é)
                | 0x2000..=0x206F    // punctuation + spaces
                | 0x2100..=0x214F    // letterlike symbols
                | 0x2190..=0x2BFF    // arrows, math, misc symbols
                | 0x2E00..=0x2E7F    // supplemental punctuation
                | 0x3000..=0x303F    // CJK punctuation
                | 0xFE50..=0xFE6F    // small form variants
                | 0xFF00..=0xFFEF    // halfwidth/fullwidth forms
            )
        }
        match c {
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            '"' => out.push('"'),
            '\x00'..='\x1f' => out.push_str(&format!("\\x{:02x}", c as u8)),
            '\x7f' => out.push_str("\\x7f"),
            c if c.is_control() => match c as u32 {
                code @ 0..=0xff => out.push_str(&format!("\\x{:02x}", code as u8)),
                code @ 0x100..=0xffff => out.push_str(&format!("\\u{:04x}", code)),
                code => out.push_str(&format!("\\U{:08x}", code)),
            },
            c if !is_printable(c) => match c as u32 {
                code @ 0x100..=0xffff => out.push_str(&format!("\\u{:04x}", code)),
                code => out.push_str(&format!("\\U{:08x}", code)),
            },
            c => out.push(c),
        }
    }
    out
}


