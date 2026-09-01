use crate::object::*;
use std::collections::HashMap;

pub fn create_string_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    let ascii_lowercase = "abcdefghijklmnopqrstuvwxyz";
    let ascii_uppercase = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    let ascii_letters = &format!("{}{}", ascii_lowercase, ascii_uppercase);
    let digits = "0123456789";
    let hexdigits = "0123456789abcdefABCDEF";
    let octdigits = "01234567";
    let punctuation = "!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~";
    let whitespace = " \t\n\r\u{0b}\u{0c}";
    let printable = &format!("{}{}{}{}", digits, ascii_letters, punctuation, whitespace);

    d.insert_str("ascii_letters", py_str(ascii_letters));
    d.insert_str("ascii_lowercase", py_str(ascii_lowercase));
    d.insert_str("ascii_uppercase", py_str(ascii_uppercase));
    d.insert_str("digits", py_str(digits));
    d.insert_str("hexdigits", py_str(hexdigits));
    d.insert_str("octdigits", py_str(octdigits));
    d.insert_str("punctuation", py_str(punctuation));
    d.insert_str("printable", py_str(printable));
    d.insert_str("whitespace", py_str(whitespace));

    d
}
