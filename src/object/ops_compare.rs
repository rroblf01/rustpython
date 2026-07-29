// Split out of the former monolithic object.rs (see the file-splitting
// refactor's memory entry for context) — this file holds the `py_compare`
// dispatcher, the `Compare` trait (`<`/`<=`/`>`/`>=`/`!=`), and the
// `NotImplemented`/`StopIteration` sentinel-recognition helpers used across
// dunder dispatch.
use super::*;

pub fn py_compare(a: &PyObjectRef, b: &PyObjectRef, op: u32) -> PyResult<PyObjectRef> {
    // Fast path for small int comparisons — no borrow() needed
    if let (Some(ai), Some(bi)) = (a.as_i64(), b.as_i64()) {
        return Ok(py_bool(match op {
            0 => ai < bi,
            1 => ai <= bi,
            2 => ai == bi,
            3 => ai >= bi,
            4 => ai > bi,
            5 => ai != bi,
            8 => a.is(b),
            9 => !a.is(b),
            _ => return Ok(py_bool(false)),
        }));
    }
    // Check for __eq__/__ne__/__lt__/__le__/__gt__/__ge__ on Instance types —
    // without this, no user-defined class's comparison operators work at
    // all (only equality was ever wired up here; ordering silently fell
    // through to the builtin Compare impl, which doesn't know Instance).
    let method_name = match op {
        0 => Some("__lt__"),
        1 => Some("__le__"),
        2 => Some("__eq__"),
        3 => Some("__ge__"),
        4 => Some("__gt__"),
        5 => Some("__ne__"),
        _ => None,
    };
    if let Some(method_name) = method_name {
        let is_a_instance = matches!(&*a.borrow(), PyObject::Instance { .. });
        let is_b_instance = matches!(&*b.borrow(), PyObject::Instance { .. });
        if is_a_instance || is_b_instance {
            if let Some(result) = try_dunder_comparison(a, b, method_name)? {
                return Ok(py_bool(result));
            }
        }
    }
    // Set/FrozenSet comparisons (subset/superset/equality relations) are
    // special-cased here — extracting clones BEFORE any borrow of `a`/`b`
    // is taken, computing the result via `PySet`'s own `is_subset`/
    // `is_superset`/`contains` on the clones — so that a hostile member's
    // `__eq__`/`__hash__` (invoked internally by those calls) can freely
    // reenter and mutate the ORIGINAL `a`/`b` without conflicting with a
    // still-held borrow. The generic `op` dispatch just below (`a.borrow().
    // lt(b)?` etc.) holds BOTH operands' borrows for its entire Set arm,
    // which panics with "RefCell already (mutably) borrowed" the instant
    // such a reentrant mutation happens — real, deliberate CPython
    // regression test: `test_set.py`'s `TestBinaryOpsMutating` (`test_lt_
    // with_mutation`, `test_eq_with_mutation`, etc., via `check_set_op_
    // does_not_crash`/`make_sets_of_bad_objects`).
    if matches!(op, 0..=5) {
        if let (Some((sa, _)), Some((sb, _))) = (extract_pyset(a), extract_pyset(b)) {
            let is_subset = sa.len() <= sb.len() && sa.is_subset(&sb);
            let is_superset = sa.len() >= sb.len() && sa.is_superset(&sb);
            let result = match op {
                0 => sa.len() < sb.len() && is_subset,
                1 => is_subset,
                2 => sa.len() == sb.len() && is_subset,
                3 => is_superset,
                4 => sa.len() > sb.len() && is_superset,
                5 => !(sa.len() == sb.len() && is_subset),
                _ => unreachable!(),
            };
            return Ok(py_bool(result));
        }
    }
    let result = match op {
        0 => a.borrow().lt(b)?,
        1 => a.borrow().le(b)?,
        2 => a.equals(b)?,
        3 => a.borrow().ge(b)?,
        4 => a.borrow().gt(b)?,
        5 => a.borrow().ne(b)?,
        6 => contains_op(b, a)?,
        7 => !contains_op(b, a)?,
        8 => a.is(b),
        9 => !a.is(b),
        _ => return Err(PyError::runtime_error("unknown comparison operator")),
    };
    Ok(py_bool(result))
}

/// True iff `v` is the `NotImplemented` singleton — a comparison dunder
/// returning it means "I don't know how to compare with this type, try the
/// other operand's reflected method instead" (or fall back to the default
/// behavior if neither side can). Without this check, any class following
/// the standard `if not isinstance(other, X): return NotImplemented`
/// idiom got NotImplemented's own truthiness (true) used as the comparison
/// result directly, e.g. making `foo == unrelated_object` always True.
pub(crate) fn is_not_implemented(v: &PyObjectRef) -> bool {
    matches!(&*v.borrow(), PyObject::Instance { typ, .. }
        if matches!(&*typ.borrow(), PyObject::Type { name, .. } if name == "NotImplementedType"))
}

/// True iff `e` signals iterator exhaustion — either the plain
/// `PyError::StopIteration` (raised internally by builtin iterators, or by
/// `raise StopIteration` with no message) or a wrapped `PyError::Exception`
/// whose type is "StopIteration" (how a generator's own `__next__`/`send`
/// driver — see the Generator match arm's get_attribute impl — signals
/// normal return/exhaustion, and how `raise StopIteration("msg")` comes out
/// of RAISE_VARARGS). Any direct caller of a generator's or custom
/// iterator's `__next__` (bypassing `builtin_next`, which already does this
/// normalization) must check both forms — checking only the bare variant
/// left FOR_ITER unable to recognize a real generator's exhaustion at all,
/// so `for x in some_generator(): ...` never terminated cleanly.
pub fn is_stop_iteration_error(e: &PyError) -> bool {
    match e {
        PyError::StopIteration => true,
        // Two distinct shapes reach here under the same enum variant:
        // (1) the generator __next__/send driver's ad hoc
        //     `PyError::Exception("StopIteration".into(), return_value)` —
        //     the second field is the generator's raw return value, not a
        //     real exception object, so only the message string identifies
        //     it; (2) a genuinely `raise StopIteration("msg")`'d exception,
        //     where the message is user text but the second field IS a
        //     real `PyObject::Exception { typ: "StopIteration", .. }`.
        PyError::Exception(msg, exc) => {
            msg == "StopIteration"
                || matches!(&*exc.borrow(), PyObject::Exception { typ, .. } if typ == "StopIteration")
        }
        _ => false,
    }
}

fn try_dunder_comparison(a: &PyObjectRef, b: &PyObjectRef, method: &str) -> PyResult<Option<bool>> {
    // Try a.__eq__(b) first
    let f_a = try_get_method(a, method);
    if let Some(f) = f_a {
        let result = call_bound_method(f, a.clone(), vec![b.clone()])?;
        if !is_not_implemented(&result) {
            return Ok(Some(result.truthy()));
        }
    }
    // Try b.__eq__(a) if different type, or if a's method punted
    if a.get_type_name() != b.get_type_name() {
        let f_b = try_get_method(b, method);
        if let Some(f) = f_b {
            let result = call_bound_method(f, b.clone(), vec![a.clone()])?;
            if !is_not_implemented(&result) {
                return Ok(Some(result.truthy()));
            }
        }
    }
    Ok(None)
}

fn try_get_method(obj: &PyObjectRef, name: &str) -> Option<PyObjectRef> {
    let typ = if let PyObject::Instance { typ, .. } = &*obj.borrow() { Some(typ.clone()) } else { None };
    // Walk the full mro (not just the instance's own exact type's dict) —
    // this is what comparison dunders (__eq__/__lt__/etc.) need to find one
    // *inherited* from a base rather than redefined on the exact class, e.g.
    // any plain class relying on `object`'s default (identity-based)
    // __eq__: without this, `try_get_method` came back None for it, so
    // comparisons fell through this function's caller entirely and hit a
    // different, separately-broken direct-identity-reconstruction path
    // instead (see PyObjectRef::equals's doc comment) — surfaced by two
    // enum members with equal `is()` identity still comparing `==` False.
    typ.and_then(|t| lookup_dunder_via_mro(&t, name))
}

pub trait Compare {
    fn lt(&self, other: &PyObjectRef) -> PyResult<bool>;
    fn le(&self, other: &PyObjectRef) -> PyResult<bool>;
    fn gt(&self, other: &PyObjectRef) -> PyResult<bool>;
    fn ge(&self, other: &PyObjectRef) -> PyResult<bool>;
    fn ne(&self, other: &PyObjectRef) -> PyResult<bool>;
}

impl Compare for PyObject {
    fn lt(&self, other: &PyObjectRef) -> PyResult<bool> {
        let other = other.borrow();
        match (self, &*other) {
            (PyObject::Int(a), PyObject::Int(b)) => Ok(a < b),
            (PyObject::Float(a), PyObject::Float(b)) => Ok(a < b),
            (PyObject::Int(a), PyObject::Float(b)) => Ok(a.to_f64().unwrap() < *b),
            (PyObject::Float(a), PyObject::Int(b)) => Ok(*a < b.to_f64().unwrap()),
            (PyObject::Str(a), PyObject::Str(b)) => Ok(a < b),
            (PyObject::Bool(a), PyObject::Bool(b)) => Ok(a < b),
            (PyObject::Bool(a), PyObject::Int(b)) => Ok((*a as i32) < b.to_i32().unwrap_or(0)),
            (PyObject::Int(a), PyObject::Bool(b)) => Ok(a.to_i32().unwrap_or(0) < (*b as i32)),
            (PyObject::Set(a), PyObject::Set(b)) => {
                // a < b: proper subset (a <= b and a != b)
                if a.len() >= b.len() { return Ok(false); }
                for item in a.to_vec() { if !b.contains(&item)? { return Ok(false); } }
                Ok(true)
            }
            (PyObject::Tuple(a), PyObject::Tuple(b)) => {
                // Route element comparison through py_compare — a raw
                // `.lt()` has no Instance/dunder dispatch, so a tuple whose
                // elements are user-defined objects with `__lt__` (a very
                // common ordering idiom: `(sort_key, obj)` tuples) always
                // raised TypeError instead of consulting it.
                for (x, y) in a.iter().zip(b.iter()) {
                    if !x.equals(y)? { return Ok(py_compare(x, y, 0)?.truthy()); }
                }
                Ok(a.len() < b.len())
            }
            (PyObject::None, PyObject::None) => Ok(false),
            _ => {
                if std::env::var("RPY_DEBUG_LT").is_ok() {
                    let self_cls = if let PyObject::Instance { typ, .. } = self { get_type_name_for_instance(typ) } else { self.type_name() };
                    let other_cls = if let PyObject::Instance { typ, .. } = &*other { get_type_name_for_instance(typ) } else { other.type_name() };
                    eprintln!("LT_FAIL: self_cls={} other_cls={}", self_cls, other_cls);
                }
                Err(PyError::type_error(format!("'<' not supported between instances of '{}' and '{}'",
                    self.type_name(), other.type_name())))
            }
        }
    }

    fn le(&self, other: &PyObjectRef) -> PyResult<bool> {
        let other = other.borrow();
        match (self, &*other) {
            (PyObject::Int(a), PyObject::Int(b)) => Ok(a <= b),
            (PyObject::Float(a), PyObject::Float(b)) => Ok(a <= b),
            (PyObject::Int(a), PyObject::Float(b)) => Ok(a.to_f64().unwrap() <= *b),
            (PyObject::Float(a), PyObject::Int(b)) => Ok(*a <= b.to_f64().unwrap()),
            (PyObject::Str(a), PyObject::Str(b)) => Ok(a <= b),
            (PyObject::Bool(a), PyObject::Bool(b)) => Ok(a <= b),
            (PyObject::Set(a), PyObject::Set(b)) => {
                if a.len() > b.len() { return Ok(false); }
                for item in a.to_vec() { if !b.contains(&item)? { return Ok(false); } }
                Ok(true)
            }
            (PyObject::Tuple(a), PyObject::Tuple(b)) => {
                for (x, y) in a.iter().zip(b.iter()) {
                    if !x.equals(y)? { return Ok(py_compare(x, y, 0)?.truthy()); }
                }
                Ok(a.len() <= b.len())
            }
            _ => Err(PyError::type_error(format!("'<=' not supported between instances of '{}' and '{}'",
                self.type_name(), other.type_name()))),
        }
    }

    fn gt(&self, other: &PyObjectRef) -> PyResult<bool> {
        let other = other.borrow();
        match (self, &*other) {
            (PyObject::Int(a), PyObject::Int(b)) => Ok(a > b),
            (PyObject::Float(a), PyObject::Float(b)) => Ok(a > b),
            (PyObject::Int(a), PyObject::Float(b)) => Ok(a.to_f64().unwrap() > *b),
            (PyObject::Float(a), PyObject::Int(b)) => Ok(*a > b.to_f64().unwrap()),
            (PyObject::Str(a), PyObject::Str(b)) => Ok(a > b),
            (PyObject::Bool(a), PyObject::Bool(b)) => Ok(a > b),
            (PyObject::Set(a), PyObject::Set(b)) => {
                if a.len() <= b.len() { return Ok(false); }
                for item in b.to_vec() { if !a.contains(&item)? { return Ok(false); } }
                Ok(true)
            }
            (PyObject::Tuple(a), PyObject::Tuple(b)) => {
                for (x, y) in a.iter().zip(b.iter()) {
                    if !x.equals(y)? { return Ok(py_compare(x, y, 4)?.truthy()); }
                }
                Ok(a.len() > b.len())
            }
            _ => Err(PyError::type_error(format!("'>' not supported between instances of '{}' and '{}'",
                self.type_name(), other.type_name()))),
        }
    }

    fn ge(&self, other: &PyObjectRef) -> PyResult<bool> {
        let other = other.borrow();
        match (self, &*other) {
            (PyObject::Int(a), PyObject::Int(b)) => Ok(a >= b),
            (PyObject::Float(a), PyObject::Float(b)) => Ok(a >= b),
            (PyObject::Int(a), PyObject::Float(b)) => Ok(a.to_f64().unwrap() >= *b),
            (PyObject::Float(a), PyObject::Int(b)) => Ok(*a >= b.to_f64().unwrap()),
            (PyObject::Str(a), PyObject::Str(b)) => Ok(a >= b),
            (PyObject::Bool(a), PyObject::Bool(b)) => Ok(a >= b),
            (PyObject::Set(a), PyObject::Set(b)) => {
                if a.len() < b.len() { return Ok(false); }
                for item in b.to_vec() { if !a.contains(&item)? { return Ok(false); } }
                Ok(true)
            }
            (PyObject::Tuple(a), PyObject::Tuple(b)) => {
                for (x, y) in a.iter().zip(b.iter()) {
                    if !x.equals(y)? { return Ok(py_compare(x, y, 4)?.truthy()); }
                }
                Ok(a.len() >= b.len())
            }
            _ => Err(PyError::type_error(format!("'>=' not supported between instances of '{}' and '{}'",
                self.type_name(), other.type_name()))),
        }
    }

    fn ne(&self, other: &PyObjectRef) -> PyResult<bool> {
        self.equals(other).map(|b| !b)
    }
}
