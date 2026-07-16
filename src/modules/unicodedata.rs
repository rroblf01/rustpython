use crate::object::*;
use std::collections::HashMap;

pub fn create_unicodedata_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! ud_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
                name: $name.to_string(),
                func: $func,
            }));
        };
    }

    ud_func!("category", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("category() requires a character"));
        }
        let s = args[0].str();
        let ch = s.chars().next().unwrap_or('\0');
        // Return the Unicode general category as a string
        let cat = match ch {
            '\u{0000}'..='\u{001F}' | '\u{007F}'..='\u{009F}' => "Cc",
            '\u{0020}' | '\u{00A0}' | '\u{1680}' | '\u{2000}'..='\u{200A}' |
            '\u{2028}' | '\u{2029}' | '\u{202F}' | '\u{205F}' | '\u{3000}' => "Zs",
            '\u{0009}' | '\u{000A}' | '\u{000B}' | '\u{000C}' | '\u{000D}' => "Cc",
            _ => {
                // Use Rust's char methods for basic category detection
                if ch.is_uppercase() { "Lu" }
                else if ch.is_lowercase() { "Ll" }
                else if ch.is_ascii_digit() { "Nd" }
                else if ch.is_whitespace() { "Zs" }
                else if ch.is_ascii_punctuation() { "Po" }
                else if ch.is_numeric() { "Nl" }
                else if ch.is_alphanumeric() { "Lo" }
                else { "So" }
            }
        };
        Ok(py_str(cat))
    });

    ud_func!("normalize", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error("normalize() requires form and unicode string"));
        }
        let _form = args[0].str();
        let text = args[1].str();
        // Simplified: pass through without actual normalization
        // Full implementation would need unicode-normalization crate
        Ok(py_str(&text))
    });

    d
}
