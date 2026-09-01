// Split from src/object/subscript.rs — `__index__` protocol helper.
use super::*;

pub fn to_index(obj: &PyObjectRef) -> PyResult<BigInt> {
    if let PyObject::WeakProxy { target, .. } = &*obj.borrow() {
        if let Some(rc) = target.upgrade() {
            return to_index(&PyObjectRef::Imm(rc));
        } else {
            return Err(PyError::reference_error("weakly-referenced object no longer exists"));
        }
    }
    let type_name = obj.get_type_name();
    let is_instance = matches!(&*obj.borrow(), PyObject::Instance { .. });
    if is_instance {
        // An int SUBCLASS (`class MyInt(int)`) uses its int VALUE directly —
        // `operator.index(MyInt(7))` is 7, NOT `MyInt.__index__()` (8) —
        // matching CPython's PyNumber_Index exact-type fast path. Only
        // non-int objects consult their `__index__`.
        if let Some(backing) = crate::object::native_backing_of(obj) {
            if let PyObject::Int(i) = &*backing.borrow() {
                return Ok(i.clone());
            }
        }
        let f = {
            let o = obj.borrow();
            match &*o {
                PyObject::Instance { typ, .. } => lookup_dunder_via_mro(typ, "__index__"),
                _ => None,
            }
        };
        if let Some(f) = f {
            let result = call_bound_method(f, obj.clone(), vec![])?;
            let r = result.borrow();
            match &*r {
                PyObject::Int(i) => Ok(i.clone()),
                // `bool` is a genuine `int` subclass in real Python, so a
                // `__index__` returning `True`/`False` is valid (if
                // deprecated in modern CPython) — matches the native-`bool`
                // arm added just below for the same reason.
                PyObject::Bool(b) => {
                    // CPython emits DeprecationWarning when __index__ returns
                    // a bool (test_index::test_index_returns_int_subclass).
                    crate::modules::warnings_emit(
                        "__index__ returned non-int (type bool)",
                        "DeprecationWarning",
                    );
                    Ok(BigInt::from(*b as i64))
                }
                _ => Err(PyError::type_error("__index__ must return int")),
            }
        } else {
            Err(PyError::type_error(format!(
                "'{}' object cannot be interpreted as an integer",
                type_name
            )))
        }
    } else {
        let o = obj.borrow();
        match &*o {
            PyObject::Int(i) => Ok(i.clone()),
            // `bool` is a subtype of `int` in real Python (`range(True) ==
            // range(1)`, `[10, 20][False]`, etc.) — found via `range()`'s
            // own `__index__`-protocol fix above surfacing this same gap.
            PyObject::Bool(b) => Ok(BigInt::from(*b as i64)),
            _ => Err(PyError::type_error(format!(
                "'{}' object cannot be interpreted as an integer",
                type_name
            ))),
        }
    }
}
