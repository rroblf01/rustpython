// Split from src/object/core.rs — NativeDispatchRecursionGuard + equals_inner + get_id.
use crate::bytecode::CodeObject;
use crate::interner::{self, StrId};
use crate::modules::*;
use crate::object::*;
use super::object_id;
use num_bigint::{BigInt, Sign};
use num_traits::{One, Signed, ToPrimitive, Zero};
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};

thread_local! {
    static NATIVE_DISPATCH_DEPTH: std::cell::Cell<usize> = std::cell::Cell::new(0);
}

/// Guards `call_bound_method`'s and `builtin_call`'s `PyObject::Function`
/// arms — both spin up a BRAND NEW disposable `VirtualMachine` (with its own
/// fresh, always-zero `self.frames`) for every single nested native-dispatch
/// call (any dunder invoked from native code: `__call__`, `__repr__`,
/// `__eq__`, ...), so `vm.rs`'s own `call_function` recursion-limit check
/// (`self.frames.len() >= self.recursion_limit`) NEVER trips for recursion
/// that flows through this path — each nesting level resets that counter to
/// zero right when a fresh VM is constructed, while the REAL native (Rust)
/// call stack keeps growing underneath, completely unbounded, until it
/// overflows for real: a hard process abort, not a catchable
/// `RecursionError`. Confirmed via CPython's own `test_descr.py`'s
/// `test_recursive_call` (`A.__call__ = A()`, then `A()()` — a textbook
/// infinite `__call__` cycle real Python catches with `RecursionError`,
/// which this interpreter instead crashed on outright). This guard is a
/// SEPARATE thread-local counter from any specific VM's own frame count —
/// tracking nesting depth across ALL disposable-VM dispatches regardless of
/// which of the two call sites (or how many alternating VMs) are involved.
/// Capped at 500, same as the `EQUALS_DEPTH`/`REPR_DEPTH` guards just above
/// — each nesting level here is more stack-expensive (constructs a whole VM
/// + frame) than one ordinary Python call frame, so a smaller cap is the
/// conservative, safe choice given the same overall native stack budget.
pub(crate) struct NativeDispatchRecursionGuard;

impl NativeDispatchRecursionGuard {
    pub(crate) fn enter() -> PyResult<Self> {
        let depth = NATIVE_DISPATCH_DEPTH.with(|c| {
            let d = c.get() + 1;
            c.set(d);
            d
        });
        if depth > 500 {
            NATIVE_DISPATCH_DEPTH.with(|c| c.set(c.get() - 1));
            return Err(PyError::recursion_error("maximum recursion depth exceeded"));
        }
        Ok(NativeDispatchRecursionGuard)
    }
}

impl Drop for NativeDispatchRecursionGuard {
    fn drop(&mut self) {
        NATIVE_DISPATCH_DEPTH.with(|c| c.set(c.get().saturating_sub(1)));
    }
}

impl PyObjectRef {
    pub(crate) fn equals_inner(&self, other: &PyObjectRef) -> PyResult<bool> {
        if let (Some(ai), Some(bi)) = (self.as_i64(), other.as_i64()) {
            return Ok(ai == bi);
        }
        // Fast path for inline floats
        if let (PyObjectRef::SmallFloat(a), PyObjectRef::SmallFloat(b)) = (self, other) {
            return Ok(a == b);
        }
        // Fast path for inline strings
        if let (PyObjectRef::SmallStr(a), PyObjectRef::SmallStr(b)) = (self, other) {
            return Ok(a.as_str() == b.as_str());
        }
        // Custom __eq__ dispatch needs THIS PyObjectRef's own identity (a
        // real Rc clone) passed as `self` — PyObject::equals below (called
        // via `.borrow()`) only has `&PyObject`, with no way to recover the
        // Rc it lives in, so it used to reconstruct a throwaway
        // `PyObjectRef::new(self.clone())` just to have something to pass
        // as `self`. That throwaway has a *different* identity than the
        // real object, so e.g. `object`'s default (identity-based) __eq__
        // always returned false — even for `x == x` on the very same
        // instance (surfaced by enum member comparisons: `Color.RED ==
        // Color.RED` came out False). Doing the mro lookup and call here,
        // with the real `self`, fixes that at the root.
        let typ = if let PyObject::Instance { typ, .. } = &*self.borrow() {
            Some(typ.clone())
        } else {
            None
        };
        let mut self_eq_not_impl = false;
        if let Some(typ) = typ {
            if let Some(f) = lookup_dunder_via_mro(&typ, "__eq__") {
                let result = call_bound_method(f, self.clone(), vec![other.clone()])?;
                if !is_not_implemented(&result) {
                    return Ok(result.truthy());
                }
                self_eq_not_impl = true;
            }
        }
        // Reflect to OTHER's __eq__ when self's own returned NotImplemented —
        // CPython: `'halibut' == HalibutProxy()` calls
        // `HalibutProxy.__eq__('halibut')`, AND `X() == Y()` where both have
        // __eq__ calls BOTH (`X.__eq__(Y())` then `Y.__eq__(X())`). Was
        // gated on self NOT being an Instance, so two custom-__eq__ objects
        // never reflected — the second __eq__ (and its side effects, e.g.
        // clearing a list — test_list::test_equal_operator_modifying_operand)
        // never ran.
        if self_eq_not_impl || !matches!(&*self.borrow(), PyObject::Instance { .. }) {
            if let PyObject::Instance { typ, .. } = &*other.borrow() {
                let typ = typ.clone();
                if let Some(f) = lookup_dunder_via_mro(&typ, "__eq__") {
                    let result = call_bound_method(f, other.clone(), vec![self.clone()])?;
                    if !is_not_implemented(&result) {
                        return Ok(result.truthy());
                    }
                }
            }
        }
        // Real CPython short-circuits container/slice `==` on POINTER
        // IDENTITY before comparing components — `s1 == s1` where a
        // component's `__eq__` raises (test_slice.py::test_cmp's `BadCmp`)
        // is True, not an exception. Bare Instances with a custom `__eq__`
        // do NOT short-circuit (`b == b` calls `__eq__` and may raise, as
        // the same test asserts).
        if !matches!(&*self.borrow(), PyObject::Instance { .. }) && self.is(other) {
            return Ok(true);
        }
        // For container types, clone elements before element-wise comparison
        // so the RefCell borrow on the container is released first. This
        // prevents RefCell panics when an element's __eq__ mutates the same
        // container during comparison (e.g. lst.index(lst) with custom __eq__
        // that calls lst.clear()).
        // `is_list` distinguishes the two so a `list` and a `tuple` with
        // identical elements don't compare equal — real Python NEVER treats
        // `list`/`tuple` as equal to each other regardless of content (only
        // to another value of the SAME container kind). The previous
        // version matched both into the same `Option<Vec<PyObjectRef>>`
        // without recording which kind `self` was, so `other`'s match arms
        // (also accepting either kind) let a `list` and a `tuple` slip
        // through as comparable — confirmed via CPython's own
        // `test_compare.py`, whose `assert_equality_only(t1, l1, False)`
        // failed because `(1, 2) == [1, 2]` came back `True`.
        let self_items = match &*self.borrow() {
            PyObject::List(items) => Some((true, items.clone())),
            PyObject::Tuple(items) => Some((false, items.clone())),
            _ => None,
        };
        if let Some((is_list, my_items)) = self_items {
            let other_items = match &*other.borrow() {
                PyObject::List(items) if is_list => Some(items.clone()),
                PyObject::Tuple(items) if !is_list => Some(items.clone()),
                _ => None,
            };
            if let Some(other_items) = other_items {
                if my_items.len() != other_items.len() {
                    return Ok(false);
                }
                // Real container `==` shortcuts each element via identity
                // BEFORE falling back to `__eq__` (`x is y or x == y`) —
                // needed for non-reflexive elements (`float('nan')`, or any
                // object whose `__eq__` always returns `False`, e.g. a
                // sentinel type). Was calling `x.equals(y)?` alone, so two
                // lists/tuples containing the exact SAME such object at
                // corresponding positions still compared unequal overall —
                // confirmed via CPython's own `test_contains.py::test_
                // nonreflexive` (`constructor(values) == constructor(values)`
                // for the SAME `values` tuple, containing both `nan` and a
                // never-equal sentinel, must be `True`).
                for (x, y) in my_items.iter().zip(other_items.iter()) {
                    if !(x.is(y) || x.equals(y)?) {
                        // bpo-38588: an element's __eq__ may MUTATE the
                        // lists mid-comparison (list1 = [X()]; list2 = [Y()];
                        // X.__eq__ clears list2, Y.__eq__ clears list1). Real
                        // CPython then re-compares the (now changed) live
                        // lists — both empty -> equal. Retry on the live
                        // contents.
                        let (live_a, live_b): (Option<Vec<PyObjectRef>>, Option<Vec<PyObjectRef>>) = {
                            let a = self.borrow();
                            let b = other.borrow();
                            (
                                match &*a {
                                    PyObject::List(l) => Some(l.clone()),
                                    _ => None,
                                },
                                match &*b {
                                    PyObject::List(l) => Some(l.clone()),
                                    _ => None,
                                },
                            )
                        };
                        if let (Some(la), Some(lb)) = (live_a, live_b) {
                            if la.len() != my_items.len() || lb.len() != other_items.len() {
                                // mutated: re-run equality on the live lists
                                // (both now empty in the bpo-38588 case).
                                if la.len() != lb.len() {
                                    return Ok(false);
                                }
                                let mut eq = true;
                                for (xa, xb) in la.iter().zip(lb.iter()) {
                                    if !(xa.is(xb) || xa.equals(xb)?) {
                                        eq = false;
                                        break;
                                    }
                                }
                                return Ok(eq);
                            }
                        }
                        return Ok(false);
                    }
                }
                return Ok(true);
            }
        }
        // Handle Dict comparison: clone items and keys to avoid RefCell conflicts
        let self_dict = match &*self.borrow() {
            PyObject::Dict(d) => Some(d.items()),
            _ => None,
        };
        if let Some(my_items) = self_dict {
            let other_dict = match &*other.borrow() {
                PyObject::Dict(d) => Some(d.items()),
                _ => None,
            };
            if let Some(other_items) = other_dict {
                if my_items.len() != other_items.len() {
                    return Ok(false);
                }
                // Same identity-shortcut fix as the List/Tuple case just
                // above, for both keys and values.
                for (k, va) in my_items {
                    let mut found = None;
                    for (ok, ov) in &other_items {
                        if ok.is(&k) || ok.equals(&k)? {
                            found = Some(ov);
                            break;
                        }
                    }
                    match found {
                        Some(vb) => {
                            if !(va.is(vb) || va.equals(vb)?) {
                                return Ok(false);
                            }
                        }
                        None => {
                            return Ok(false);
                        }
                    }
                }
                return Ok(true);
            }
        }
        self.borrow().equals(other)
    }
    pub fn get_type_name(&self) -> String {
        self.borrow().type_name()
    }

    pub fn get_id(&self) -> usize {
        match self {
            PyObjectRef::None => object_id::none_id(),
            PyObjectRef::SmallBool(b) => object_id::bool_id(*b),
            PyObjectRef::SmallInt(n) => object_id::int_id(*n),
            PyObjectRef::SmallFloat(f) => object_id::float_id(f.to_bits()),
            // `SmallStr` keeps the OLD (already-broken, unstable) behavior
            // deliberately — unlike None/bool/int/float, `PyObjectRef::is`
            // does NOT treat two equal-content `SmallStr`s as identity-equal
            // (falls to the catch-all `_ => false`), so giving them a
            // value-derived id here would make `id(a) == id(b)` disagree
            // with `a is b` in the OTHER direction. Fixing this properly
            // needs a decision about `SmallStr` identity semantics first,
            // not just an `id()` patch — left as a known, separate,
            // lower-priority gap (real CPython doesn't guarantee string
            // interning either, so this is a quirk, not a correctness bug
            // against the language spec).
            PyObjectRef::SmallStr(_) => self as *const PyObjectRef as usize,
            PyObjectRef::Mut(rc) => object_id::heap_id(Rc::as_ptr(rc) as usize),
            PyObjectRef::Imm(rc) => object_id::heap_id(Rc::as_ptr(rc) as usize),
        }
    }
}
