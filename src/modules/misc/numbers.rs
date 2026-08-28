use crate::object::*;
use std::collections::HashMap;

pub fn create_numbers_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    // Number ABCs — simple string stubs (matchable via isinstance checks later)
    d.insert_str("Number", py_str("Number"));
    d.insert_str("Complex", py_str("Complex"));
    d.insert_str("Real", py_str("Real"));
    d.insert_str("Rational", py_str("Rational"));
    d.insert_str("Integral", py_str("Integral"));
    d
}
