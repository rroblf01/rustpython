use crate::object::*;
#[allow(unused_imports)]
use num_traits::{Signed, ToPrimitive};
#[allow(unused_imports)]
use std::collections::HashMap;

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

/// Run the registered codec search functions (in registration order) until
/// one claims `encoding`, returning that function's 4-tuple
/// `(name, encode, decode, errors)`. `Ok(None)` means no search function
/// claimed the codec.
pub fn lookup_codec(encoding: &str) -> crate::object::PyResult<Option<PyObjectRef>> {
    let mut outcome: Option<crate::object::PyResult<PyObjectRef>> = None;
    super::CODEC_SEARCH_FUNCTIONS.with(|fns| {
        for f in fns.borrow().iter() {
            let res = match crate::object::builtin_call(f, &[py_str(encoding)]) {
                Ok(res) => res,
                Err(_) => continue,
            };
            if matches!(&*res.borrow(), PyObject::None) {
                continue;
            }
            // CPython's `_PyCodec_Lookup` (Python/codecs.c): a search
            // function that returns anything other than None must return a
            // 4-tuple `(name, encode, decode, errors)`; anything else is a
            // TypeError. Without this check a bogus search function (e.g.
            // `unittest.mock.Mock`, which returns a truthy Mock for every
            // name) silently became the "codec" — `getreader()` then handed
            // back a Mock whose `.readline()` never returns '', spinning
            // `while line: line = reader.readline()` forever and hanging the
            // whole test file.
            let is_4tuple = tuple_len_is(&res, 4);
            outcome = Some(if is_4tuple {
                Ok(res)
            } else {
                Err(crate::object::PyError::type_error(
                    "codec search functions must return 4-tuples",
                ))
            });
            break;
        }
    });
    // Borrow released above; fold in any re-entrant registrations.
    super::drain_pending_search_functions();
    match outcome {
        Some(Ok(res)) => Ok(Some(res)),
        Some(Err(e)) => Err(e),
        None => Ok(None),
    }
}

/// True when `obj` is a `tuple` (including a tuple SUBCLASS such as
/// `codecs.CodecInfo`) of exactly `want` elements. Asked through the normal
/// `isinstance`/`len` builtins on purpose: a `CodecInfo` built by the
/// vendored `Lib/codecs.py` is a Python-level tuple subclass, so the Rust
/// `PyObject::Tuple` variant does not describe it.
fn tuple_len_is(obj: &PyObjectRef, want: usize) -> bool {
    let tuple_type = match crate::modules::get_builtin_class("tuple") {
        Some(t) => t,
        None => return false,
    };
    let is_tuple = crate::object::builtin_isinstance(&[obj.clone(), tuple_type])
        .map(|r| r.truthy())
        .unwrap_or(false);
    if !is_tuple {
        return false;
    }
    match crate::object::builtin_len(&[obj.clone()]) {
        Ok(n) => n.str() == want.to_string(),
        // A tuple subclass whose type overrides no `__len__` (and whose
        // native backing cannot answer one) still counts: it is a tuple, and
        // only the tuple-ness matters for rejecting non-codec search
        // functions.
        Err(_) => true,
    }
}
