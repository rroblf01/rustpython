// Split out of the former monolithic object.rs (see the file-splitting
// refactor's memory entry for context) — this file holds the `py_compare`
// dispatcher, the `Compare` trait (`<`/`<=`/`>`/`>=`/`!=`), and the
// `NotImplemented`/`StopIteration` sentinel-recognition helpers used across
// dunder dispatch.
use super::*;

pub fn py_compare(a: &PyObjectRef, b: &PyObjectRef, op: u32) -> PyResult<PyObjectRef> {
    if std::env::var("RPY_DEBUG_NUMERIC").is_ok() && matches!(op, 0 | 1 | 3 | 4) {
        eprintln!("PY_COMPARE op={} a={} b={} a_type={} b_type={}", op, a.borrow().repr(), b.borrow().repr(), a.get_type_name(), b.get_type_name());
    }
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
    // `try_rich_compare` implements real CPython's actual dispatch
    // algorithm (subclass-reflected-priority, each dunder called at most
    // once, `NotImplemented` on both sides falls back to identity for
    // eq/ne or `TypeError` for ordering) — see its own doc comment.
    if matches!(op, 0..=5) {
        let is_a_instance = matches!(&*a.borrow(), PyObject::Instance { .. });
        let is_b_instance = matches!(&*b.borrow(), PyObject::Instance { .. });
        if std::env::var("RPY_DEBUG_NUMERIC").is_ok() && matches!(op, 0 | 1 | 3 | 4) {
            eprintln!("INSTANCE CHECK: op={} a_is_instance={} b_is_instance={} a={} b={}", op, is_a_instance, is_b_instance, a.borrow().repr(), b.borrow().repr());
        }
        if is_a_instance || is_b_instance {
            if let Some(result) = try_rich_compare(a, b, op)? {
                return Ok(result);
            }
            // A class transparently subclassing a native container
            // (`class MyList(list): pass`) with NO explicit comparison
            // dunders anywhere in its own mro falls through to here —
            // `list`/`str`/`tuple`/etc themselves (now real, migrated
            // `Type` objects) have no ACTUAL Python-callable `__eq__`/
            // `__lt__`/etc stored in their type-dict (native comparison is
            // implemented via the separate `PyObject::equals`/`Compare`
            // machinery below, never as a real dunder `lookup_dunder_via_
            // mro` could find), so `try_rich_compare` always returned
            // `None` here and this fell all the way to raw IDENTITY
            // comparison for `==`/`!=` (`TypeError` for ordering) —
            // `MyList([1,2]) == MyList([1,2])` was always `False` unless
            // the two were the literal same object, and e.g. a `str`
            // subclass instance compared with `<` against another raised
            // `TypeError` outright even though real `str` is totally
            // ordered and a subclass inherits that (confirmed via
            // `test_xml_dom_minicompat.py`'s `NodeList(list)` and
            // `test_compare.py::test_str_subclass`). Delegate to the
            // NATIVE BACKING by recursing into `py_compare` itself with
            // the unwrapped native value(s) — reuses ALL of its existing
            // native comparison logic (list/tuple/str/bytes ordering, set
            // subset/superset semantics, ...) instead of duplicating any
            // of it here.
            let a_native = native_backing_of(a);
            let b_native = native_backing_of(b);
            if a_native.is_some() || b_native.is_some() {
                let a_cmp = a_native.unwrap_or_else(|| a.clone());
                let b_cmp = b_native.unwrap_or_else(|| b.clone());
                return py_compare(&a_cmp, &b_cmp, op);
            }
            // A `fractions.Fraction`/`decimal.Decimal` INSTANCE compared
            // against a native complex/int/float (or another Fraction/
            // Decimal) — real CPython's numeric tower compares these by
            // VALUE (`Fraction(2002,2) == 1001+0j`, `Decimal('1001.0') ==
            // 1001`). `try_rich_compare` returned None (Fraction/Decimal
            // have no __eq__ for these operand types), so without this the
            // fallback below would return raw IDENTITY for ==/!=.
            // For ordering (<, <=, >, >=) use the same numeric value (f64)
            // comparison when both sides are real numbers (imag == 0).
            let ap_opt = crate::modules::numeric_parts_from_ref(a);
            let bp_opt = crate::modules::numeric_parts_from_ref(b);
            if std::env::var("RPY_DEBUG_NUMERIC").is_ok() && matches!(op, 0 | 1 | 3 | 4) {
                eprintln!("NUMERIC DEBUG: op={} a={} b={} ap={:?} bp={:?}", op, a.borrow().repr(), b.borrow().repr(), ap_opt, bp_opt);
            }
            if let (Some(ap), Some(bp)) = (ap_opt, bp_opt) {
                // Equality / inequality always works via numeric parts
                if matches!(op, 2 | 5) {
                    let eq = ap == bp;
                    return Ok(py_bool(if op == 2 { eq } else { !eq }));
                }
                // Ordering: only for real numbers (imag == 0 for both) and
                // neither is a Complex (complex ordering must raise TypeError
                // even for 1+0j, per test_numeric_tower.test_complex)
                let a_is_complex = matches!(&*a.borrow(), PyObject::Complex(_, _));
                let b_is_complex = matches!(&*b.borrow(), PyObject::Complex(_, _));
                if matches!(op, 0 | 1 | 3 | 4) && ap.1 == 0.0 && bp.1 == 0.0 && !a_is_complex && !b_is_complex {
                    let ord = ap.0.partial_cmp(&bp.0);
                    let result = match (op, ord) {
                        (0, Some(std::cmp::Ordering::Less)) => true,
                        (0, _) => false,
                        (1, Some(std::cmp::Ordering::Less)) | (1, Some(std::cmp::Ordering::Equal)) => true,
                        (1, _) => false,
                        (3, Some(std::cmp::Ordering::Greater)) | (3, Some(std::cmp::Ordering::Equal)) => true,
                        (3, _) => false,
                        (4, Some(std::cmp::Ordering::Greater)) => true,
                        (4, _) => false,
                        _ => false,
                    };
                    // NaN ordering: always false for <,>,<=,>= (like CPython)
                    return Ok(py_bool(result));
                }
            }
            return Ok(py_bool(match op {
                2 => a.is(b),
                5 => !a.is(b),
                _ => {
                    let op_sym = match op {
                        0 => "<",
                        1 => "<=",
                        3 => ">=",
                        4 => ">",
                        _ => unreachable!(),
                    };
                    return Err(PyError::type_error(format!(
                        "'{}' not supported between instances of '{}' and '{}'",
                        op_sym,
                        a.get_type_name(),
                        b.get_type_name()
                    )));
                }
            }));
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
    // `list` ordering (`<`/`<=`/`>=`/`>`) needs the SAME clone-before-compare
    // treatment as the `Set` case just above, for the same reason: unlike
    // `Tuple` (immutable, so this hazard can't arise), a `list`'s own
    // elements can have a hostile `__eq__`/`__lt__` that mutates the very
    // list being compared (`self`/`other`) mid-comparison. Going through
    // `PyObject::lt`/`le`/`gt`/`ge`'s `List` arm (via `a.borrow().lt(b)?`
    // below) holds a live borrow on `a` for the ENTIRE comparison, so that
    // reentrant mutation panics with "RefCell already borrowed" — confirmed
    // via CPython's own `test_list.py` (a real regression test for exactly
    // this scenario, mirroring `test_set.py`'s `TestBinaryOpsMutating`
    // already handled by the `Set` block above). Handling `List` here
    // instead — cloning both operands' elements up front, before any
    // comparison call that could reenter — sidesteps the hazard entirely.
    if matches!(op, 0 | 1 | 3 | 4) {
        let a_items = if let PyObject::List(v) = &*a.borrow() {
            Some(v.clone())
        } else {
            None
        };
        if let Some(a_items) = a_items {
            let b_items = if let PyObject::List(v) = &*b.borrow() {
                Some(v.clone())
            } else {
                None
            };
            if let Some(b_items) = b_items {
                let mut ord = std::cmp::Ordering::Equal;
                for (x, y) in a_items.iter().zip(b_items.iter()) {
                    if !x.equals(y)? {
                        ord = if py_compare(x, y, 0)?.truthy() {
                            std::cmp::Ordering::Less
                        } else {
                            std::cmp::Ordering::Greater
                        };
                        break;
                    }
                }
                if ord == std::cmp::Ordering::Equal {
                    ord = a_items.len().cmp(&b_items.len());
                }
                let result = match op {
                    0 => ord == std::cmp::Ordering::Less,
                    1 => ord != std::cmp::Ordering::Greater,
                    3 => ord != std::cmp::Ordering::Less,
                    4 => ord == std::cmp::Ordering::Greater,
                    _ => unreachable!(),
                };
                return Ok(py_bool(result));
            }
        }
        // `deque` ordering gets the same clone-before-compare treatment as
        // `List` just above (its own elements can carry a mutating `__eq__`
        // that reenters and mutates the deque being compared mid-comparison).
        let a_items = if let PyObject::Deque { data, .. } = &*a.borrow() {
            Some(data.clone())
        } else {
            None
        };
        if let Some(a_items) = a_items {
            let b_items = if let PyObject::Deque { data, .. } = &*b.borrow() {
                Some(data.clone())
            } else {
                None
            };
            if let Some(b_items) = b_items {
                let mut ord = std::cmp::Ordering::Equal;
                for (x, y) in a_items.iter().zip(b_items.iter()) {
                    if !x.equals(y)? {
                        ord = if py_compare(x, y, 0)?.truthy() {
                            std::cmp::Ordering::Less
                        } else {
                            std::cmp::Ordering::Greater
                        };
                        break;
                    }
                }
                if ord == std::cmp::Ordering::Equal {
                    ord = a_items.len().cmp(&b_items.len());
                }
                let result = match op {
                    0 => ord == std::cmp::Ordering::Less,
                    1 => ord != std::cmp::Ordering::Greater,
                    3 => ord != std::cmp::Ordering::Less,
                    4 => ord == std::cmp::Ordering::Greater,
                    _ => unreachable!(),
                };
                return Ok(py_bool(result));
            }
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

/// Implements real CPython's actual rich-comparison dispatch algorithm
/// (`do_richcompare` in `object.c`) for a comparison involving at least one
/// `Instance` operand: each side's dunder is called AT MOST ONCE, with the
/// "subclass reflected priority" rule — if `b`'s type is a PROPER subclass
/// of `a`'s type (and different from it), `b`'s reflected method is tried
/// FIRST, before `a`'s own. Returns `None` if neither side's dunder ever
/// produced a definite (non-`NotImplemented`) answer, leaving the identity-
/// based eq/ne fallback (or `TypeError` for ordering) to the caller.
///
/// This REPLACES the previous `try_dunder_comparison`, which called
/// `a`'s method, and on `NotImplemented` fell through to a SEPARATE,
/// independent dispatch path (`Compare`/`equals`) that redundantly
/// re-walked the mro and re-called the SAME dunder a second (sometimes
/// third) time — confirmed via a direct repro: a custom `__eq__` returning
/// `NotImplemented` was invoked 2-3 times for a single `==` instead of
/// once, and `object`'s own default `__eq__`/`__ne__` (identity-based, no
/// `NotImplemented` support at the time) meant a subclass's real override
/// was sometimes never even reached — real trigger: CPython's own
/// `test_compare.py` (`test_ne_high_priority`/`test_ne_low_priority`/
/// `test_other_delegation`, which assert the EXACT sequence and count of
/// dunder calls made).
fn try_rich_compare(a: &PyObjectRef, b: &PyObjectRef, op: u32) -> PyResult<Option<PyObjectRef>> {
    let (own_name, refl_name) = match op {
        0 => ("__lt__", "__gt__"),
        1 => ("__le__", "__ge__"),
        2 => ("__eq__", "__eq__"),
        3 => ("__ge__", "__le__"),
        4 => ("__gt__", "__lt__"),
        5 => ("__ne__", "__ne__"),
        _ => return Ok(None),
    };

    let instance_type_of = |v: &PyObjectRef| -> Option<PyObjectRef> {
        if let PyObject::Instance { typ, .. } = &*v.borrow() {
            Some(typ.clone())
        } else {
            None
        }
    };
    let is_proper_subclass = |sub: &PyObjectRef, base: &PyObjectRef| -> bool {
        if sub.is(base) {
            return false;
        }
        if let PyObject::Type { mro, .. } = &*sub.borrow() {
            mro.iter().any(|c| c.is(base))
        } else {
            false
        }
    };
    let try_side = |self_ref: &PyObjectRef,
                    other_ref: &PyObjectRef,
                    method: &str|
     -> PyResult<Option<PyObjectRef>> {
        // Extract (clone) `typ` and drop the borrow BEFORE calling
        // `call_bound_method` — `typ` used to stay borrowed from
        // `self_ref` for the whole `if let` block (a `&*self_ref.borrow()`
        // temporary lives as long as the binding it produced), which is
        // still live while the found dunder method's body actually runs.
        // A method that mutates `self` (any ordinary `self.attr = ...`,
        // e.g. `test_collections.py`'s own comparison-mixin test helper:
        // `class Other: def __eq__(self, other): self.right_side = True;
        // return True`, aliased as `__lt__`/`__gt__`/etc. too) then hit a
        // STORE_ATTR needing `self_ref.borrow_mut()` while THIS closure
        // still held an outstanding `.borrow()` on the very same object —
        // a guaranteed double-borrow panic. Real trigger: real
        // `Lib/_collections_abc.py`'s `Mapping.__eq__` finally being a
        // genuine Python method (the old native `collections.abc` stub's
        // comparisons never exercised this path the same way) made
        // `test_collections.py`'s `TestCollectionABCs.test_Mapping` reach
        // this reflected-operator dispatch for the first time.
        let typ = match &*self_ref.borrow() {
            PyObject::Instance { typ, .. } => Some(typ.clone()),
            _ => None,
        };
        if let Some(typ) = typ {
            if let Some(f) = lookup_dunder_via_mro(&typ, method) {
                let result = call_bound_method(f, self_ref.clone(), vec![other_ref.clone()])?;
                if !is_not_implemented(&result) {
                    // Return the RAW dunder result — real CPython's rich
                    // comparison returns it as-is (test_bool's Symbol.__gt__
                    // returns a SymbolicBool, which the `if` then truth-tests,
                    // propagating its raising __bool__); converting here with
                    // truthy() swallowed that error.
                    return Ok(Some(result));
                }
            }
        }
        Ok(None)
    };
    // For `!=`, a class with no `__ne__` uses `object.__ne__`'s default:
    // `not (self.__eq__(other))` — but a class that DEFINES `__ne__` gets
    // only that called (a `NotImplemented` return defers to the reflected
    // side, never to its own `__eq__`).
    let try_ne_side =
        |self_ref: &PyObjectRef, other_ref: &PyObjectRef| -> PyResult<Option<PyObjectRef>> {
            // See `try_side`'s doc comment just above for why `typ` must be
            // cloned and the borrow dropped before calling into Python —
            // same fix, same reason, for both dunders this tries.
            let typ = match &*self_ref.borrow() {
                PyObject::Instance { typ, .. } => Some(typ.clone()),
                _ => None,
            };
            if let Some(typ) = typ {
                if let Some(f) = lookup_dunder_via_mro(&typ, "__ne__") {
                    let result = call_bound_method(f, self_ref.clone(), vec![other_ref.clone()])?;
                    if !is_not_implemented(&result) {
                        return Ok(Some(result));
                    }
                } else if let Some(f) = lookup_dunder_via_mro(&typ, "__eq__") {
                    let result = call_bound_method(f, self_ref.clone(), vec![other_ref.clone()])?;
                    if !is_not_implemented(&result) {
                        return Ok(Some(py_bool(!result.truthy())));
                    }
                }
            }
            Ok(None)
        };

    let a_type = instance_type_of(a);
    let b_type = instance_type_of(b);
    let b_first = match (&a_type, &b_type) {
        (Some(at), Some(bt)) => is_proper_subclass(bt, at),
        _ => false,
    };

    if b_first {
        if let Some(r) = if op == 5 {
            try_ne_side(b, a)?
        } else {
            try_side(b, a, refl_name)?
        } {
            return Ok(Some(r));
        }
        if let Some(r) = if op == 5 {
            try_ne_side(a, b)?
        } else {
            try_side(a, b, own_name)?
        } {
            return Ok(Some(r));
        }
    } else {
        if let Some(r) = if op == 5 {
            try_ne_side(a, b)?
        } else {
            try_side(a, b, own_name)?
        } {
            return Ok(Some(r));
        }
        if let Some(r) = if op == 5 {
            try_ne_side(b, a)?
        } else {
            try_side(b, a, refl_name)?
        } {
            return Ok(Some(r));
        }
    }
    Ok(None)
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
            (PyObject::Float(a), PyObject::Bool(b)) => Ok(*a < if *b { 1.0 } else { 0.0 }),
            (PyObject::Bool(a), PyObject::Float(b)) => Ok(if *a { 1.0 } else { 0.0 } < *b),
            (PyObject::Str(a), PyObject::Str(b)) => Ok(a < b),
            (PyObject::Bool(a), PyObject::Bool(b)) => Ok(a < b),
            (PyObject::Bool(a), PyObject::Int(b)) => Ok((*a as i32) < b.to_i32().unwrap_or(0)),
            (PyObject::Int(a), PyObject::Bool(b)) => Ok(a.to_i32().unwrap_or(0) < (*b as i32)),
            (PyObject::Set(a), PyObject::Set(b)) => {
                // a < b: proper subset (a <= b and a != b)
                if a.len() >= b.len() {
                    return Ok(false);
                }
                for item in a.to_vec() {
                    if !b.contains(&item)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            (PyObject::Tuple(a), PyObject::Tuple(b)) => {
                // Route element comparison through py_compare — a raw
                // `.lt()` has no Instance/dunder dispatch, so a tuple whose
                // elements are user-defined objects with `__lt__` (a very
                // common ordering idiom: `(sort_key, obj)` tuples) always
                // raised TypeError instead of consulting it.
                for (x, y) in a.iter().zip(b.iter()) {
                    if !x.equals(y)? {
                        return Ok(py_compare(x, y, 0)?.truthy());
                    }
                }
                Ok(a.len() < b.len())
            }
            // `list`/`bytes`/`bytearray` ordering was entirely missing —
            // only Tuple got the lexicographic-comparison treatment above,
            // so `[1,2] < [1,3]` or `b"a" < b"b"` raised `TypeError`
            // outright (confirmed via CPython's own `test_compare.py`,
            // whose whole point is exercising exactly this). `bytes`/
            // `bytearray` are plain `Vec<u8>`, which Rust already orders
            // lexicographically by byte value — real Python allows
            // comparing across the two types too. `list` needs the same
            // dunder-aware elementwise walk as `Tuple` just above (elements
            // may be user objects with `__lt__`).
            (PyObject::Bytes(a), PyObject::Bytes(b)) => Ok(a < b),
            (PyObject::ByteArray(a), PyObject::ByteArray(b)) => Ok(a < b),
            (PyObject::Bytes(a), PyObject::ByteArray(b)) => Ok(a.as_slice() < b.as_slice()),
            (PyObject::ByteArray(a), PyObject::Bytes(b)) => Ok(a.as_slice() < b.as_slice()),
            (PyObject::List(a), PyObject::List(b)) => {
                for (x, y) in a.iter().zip(b.iter()) {
                    if !x.equals(y)? {
                        return Ok(py_compare(x, y, 0)?.truthy());
                    }
                }
                Ok(a.len() < b.len())
            }
            (PyObject::Deque { data: a, .. }, PyObject::Deque { data: b, .. }) => {
                for (x, y) in a.iter().zip(b.iter()) {
                    if !x.equals(y)? {
                        return Ok(py_compare(x, y, 0)?.truthy());
                    }
                }
                Ok(a.len() < b.len())
            }
            (PyObject::None, PyObject::None) => Ok(false),
            _ => {
                if std::env::var("RPY_DEBUG_LT").is_ok() {
                    let self_cls = if let PyObject::Instance { typ, .. } = self {
                        get_type_name_for_instance(typ)
                    } else {
                        self.type_name()
                    };
                    let other_cls = if let PyObject::Instance { typ, .. } = &*other {
                        get_type_name_for_instance(typ)
                    } else {
                        other.type_name()
                    };
                    eprintln!("LT_FAIL: self_cls={} other_cls={}", self_cls, other_cls);
                }
                Err(PyError::type_error(format!(
                    "'<' not supported between instances of '{}' and '{}'",
                    self.type_name(),
                    other.type_name()
                )))
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
            (PyObject::Float(a), PyObject::Bool(b)) => Ok(*a <= if *b { 1.0 } else { 0.0 }),
            (PyObject::Bool(a), PyObject::Float(b)) => Ok(if *a { 1.0 } else { 0.0 } <= *b),
            (PyObject::Str(a), PyObject::Str(b)) => Ok(a <= b),
            (PyObject::Bool(a), PyObject::Bool(b)) => Ok(a <= b),
            (PyObject::Set(a), PyObject::Set(b)) => {
                if a.len() > b.len() {
                    return Ok(false);
                }
                for item in a.to_vec() {
                    if !b.contains(&item)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            (PyObject::Tuple(a), PyObject::Tuple(b)) => {
                for (x, y) in a.iter().zip(b.iter()) {
                    if !x.equals(y)? {
                        return Ok(py_compare(x, y, 0)?.truthy());
                    }
                }
                Ok(a.len() <= b.len())
            }
            (PyObject::Bytes(a), PyObject::Bytes(b)) => Ok(a <= b),
            (PyObject::ByteArray(a), PyObject::ByteArray(b)) => Ok(a <= b),
            (PyObject::Bytes(a), PyObject::ByteArray(b)) => Ok(a.as_slice() <= b.as_slice()),
            (PyObject::ByteArray(a), PyObject::Bytes(b)) => Ok(a.as_slice() <= b.as_slice()),
            (PyObject::List(a), PyObject::List(b)) => {
                for (x, y) in a.iter().zip(b.iter()) {
                    if !x.equals(y)? {
                        return Ok(py_compare(x, y, 0)?.truthy());
                    }
                }
                Ok(a.len() <= b.len())
            }
            (PyObject::Deque { data: a, .. }, PyObject::Deque { data: b, .. }) => {
                for (x, y) in a.iter().zip(b.iter()) {
                    if !x.equals(y)? {
                        return Ok(py_compare(x, y, 0)?.truthy());
                    }
                }
                Ok(a.len() <= b.len())
            }
            _ => Err(PyError::type_error(format!(
                "'<=' not supported between instances of '{}' and '{}'",
                self.type_name(),
                other.type_name()
            ))),
        }
    }

    fn gt(&self, other: &PyObjectRef) -> PyResult<bool> {
        let other = other.borrow();
        match (self, &*other) {
            (PyObject::Int(a), PyObject::Int(b)) => Ok(a > b),
            (PyObject::Float(a), PyObject::Float(b)) => Ok(a > b),
            (PyObject::Int(a), PyObject::Float(b)) => Ok(a.to_f64().unwrap() > *b),
            (PyObject::Float(a), PyObject::Int(b)) => Ok(*a > b.to_f64().unwrap()),
            (PyObject::Float(a), PyObject::Bool(b)) => Ok(*a > if *b { 1.0 } else { 0.0 }),
            (PyObject::Bool(a), PyObject::Float(b)) => Ok(if *a { 1.0 } else { 0.0 } > *b),
            (PyObject::Str(a), PyObject::Str(b)) => Ok(a > b),
            (PyObject::Bool(a), PyObject::Bool(b)) => Ok(a > b),
            (PyObject::Set(a), PyObject::Set(b)) => {
                if a.len() <= b.len() {
                    return Ok(false);
                }
                for item in b.to_vec() {
                    if !a.contains(&item)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            (PyObject::Tuple(a), PyObject::Tuple(b)) => {
                for (x, y) in a.iter().zip(b.iter()) {
                    if !x.equals(y)? {
                        return Ok(py_compare(x, y, 4)?.truthy());
                    }
                }
                Ok(a.len() > b.len())
            }
            (PyObject::Bytes(a), PyObject::Bytes(b)) => Ok(a > b),
            (PyObject::ByteArray(a), PyObject::ByteArray(b)) => Ok(a > b),
            (PyObject::Bytes(a), PyObject::ByteArray(b)) => Ok(a.as_slice() > b.as_slice()),
            (PyObject::ByteArray(a), PyObject::Bytes(b)) => Ok(a.as_slice() > b.as_slice()),
            (PyObject::List(a), PyObject::List(b)) => {
                for (x, y) in a.iter().zip(b.iter()) {
                    if !x.equals(y)? {
                        return Ok(py_compare(x, y, 4)?.truthy());
                    }
                }
                Ok(a.len() > b.len())
            }
            (PyObject::Deque { data: a, .. }, PyObject::Deque { data: b, .. }) => {
                for (x, y) in a.iter().zip(b.iter()) {
                    if !x.equals(y)? {
                        return Ok(py_compare(x, y, 4)?.truthy());
                    }
                }
                Ok(a.len() > b.len())
            }
            _ => Err(PyError::type_error(format!(
                "'>' not supported between instances of '{}' and '{}'",
                self.type_name(),
                other.type_name()
            ))),
        }
    }

    fn ge(&self, other: &PyObjectRef) -> PyResult<bool> {
        let other = other.borrow();
        match (self, &*other) {
            (PyObject::Int(a), PyObject::Int(b)) => Ok(a >= b),
            (PyObject::Float(a), PyObject::Float(b)) => Ok(a >= b),
            (PyObject::Int(a), PyObject::Float(b)) => Ok(a.to_f64().unwrap() >= *b),
            (PyObject::Float(a), PyObject::Int(b)) => Ok(*a >= b.to_f64().unwrap()),
            (PyObject::Float(a), PyObject::Bool(b)) => Ok(*a >= if *b { 1.0 } else { 0.0 }),
            (PyObject::Bool(a), PyObject::Float(b)) => Ok(if *a { 1.0 } else { 0.0 } >= *b),
            (PyObject::Str(a), PyObject::Str(b)) => Ok(a >= b),
            (PyObject::Bool(a), PyObject::Bool(b)) => Ok(a >= b),
            (PyObject::Set(a), PyObject::Set(b)) => {
                if a.len() < b.len() {
                    return Ok(false);
                }
                for item in b.to_vec() {
                    if !a.contains(&item)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            (PyObject::Tuple(a), PyObject::Tuple(b)) => {
                for (x, y) in a.iter().zip(b.iter()) {
                    if !x.equals(y)? {
                        return Ok(py_compare(x, y, 4)?.truthy());
                    }
                }
                Ok(a.len() >= b.len())
            }
            (PyObject::Bytes(a), PyObject::Bytes(b)) => Ok(a >= b),
            (PyObject::ByteArray(a), PyObject::ByteArray(b)) => Ok(a >= b),
            (PyObject::Bytes(a), PyObject::ByteArray(b)) => Ok(a.as_slice() >= b.as_slice()),
            (PyObject::ByteArray(a), PyObject::Bytes(b)) => Ok(a.as_slice() >= b.as_slice()),
            (PyObject::List(a), PyObject::List(b)) => {
                for (x, y) in a.iter().zip(b.iter()) {
                    if !x.equals(y)? {
                        return Ok(py_compare(x, y, 4)?.truthy());
                    }
                }
                Ok(a.len() >= b.len())
            }
            (PyObject::Deque { data: a, .. }, PyObject::Deque { data: b, .. }) => {
                for (x, y) in a.iter().zip(b.iter()) {
                    if !x.equals(y)? {
                        return Ok(py_compare(x, y, 4)?.truthy());
                    }
                }
                Ok(a.len() >= b.len())
            }
            _ => Err(PyError::type_error(format!(
                "'>=' not supported between instances of '{}' and '{}'",
                self.type_name(),
                other.type_name()
            ))),
        }
    }

    fn ne(&self, other: &PyObjectRef) -> PyResult<bool> {
        self.equals(other).map(|b| !b)
    }
}
