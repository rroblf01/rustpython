use crate::object::*;
#[allow(unused_imports)]
use std::collections::HashMap;
#[allow(unused_imports)]
use num_traits::{Signed, ToPrimitive};

pub(crate) fn _codecs_charmap_build(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error(
            "charmap_build() requires at least 1 argument",
        ));
    }
    let s = args[0].str();
    let chars: Vec<char> = s.chars().collect();
    let mut dict = PyDict::new();
    for (i, &ch) in chars.iter().enumerate() {
        if i >= 256 {
            break;
        }
        if ch == '\u{FFFE}' {
            continue;
        }
        let cp = ch as u32;
        let key = py_int(cp as i64);
        let val = py_int(i as i64);
        let existing = dict.get(&key).ok().flatten();
        if existing.is_some() {
            let _ = dict.set(key, py_none());
        } else {
            let _ = dict.set(key, val);
        }
    }
    Ok(PyObjectRef::new(PyObject::Dict(Box::new(dict))))
}

pub fn lookup_codec(encoding: &str) -> Option<PyObjectRef> {
    let candidate = super::CODEC_SEARCH_FUNCTIONS.with(|fns| {
        for f in fns.borrow().iter() {
            match crate::object::builtin_call(f, &[py_str(encoding)]) {
                Ok(res) if !matches!(&*res.borrow(), PyObject::None) => return Some(res),
                _ => continue,
            }
        }
        None
    });
    candidate
}
