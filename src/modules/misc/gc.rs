use crate::object::*;
use std::collections::HashMap;

pub fn create_gc_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! gc_func {
        ($name:expr, $func:expr) => {
            d.insert(
                $name.to_string(),
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: $name.to_string(),
                    func: $func,
                }),
            );
        };
    }

    // Wire gc.collect() to the real cycle collector in cycle_gc.rs — this
    // runs unconditionally (not feature-gated) since it operates on the
    // actual `Rc<RefCell<PyObject>>`-based object model every build uses,
    // unlike `gc.rs`'s separate experimental tracing heap (feature `gc`,
    // not wired into the object model at all).
    gc_func!("collect", |args| {
        let collected = crate::cycle_gc::collect();
        crate::modules::misc::run_weakref_callbacks();
        let _ = crate::object::with_vm_mut(|vm| vm.run_pending_finalizers());
        let _ = args;
        Ok(py_int(collected as i64))
    });

    gc_func!("enable", |_| {
        crate::cycle_gc::set_enabled(true);
        Ok(py_none())
    });

    gc_func!("disable", |_| {
        crate::cycle_gc::set_enabled(false);
        Ok(py_none())
    });

    gc_func!("isenabled", |_| {
        Ok(py_bool(crate::cycle_gc::is_enabled()))
    });

    gc_func!("get_count", |_| {
        let (tracked, _) = crate::cycle_gc::stats();
        Ok(py_tuple(vec![py_int(tracked as i64), py_int(0), py_int(0)]))
    });

    gc_func!("is_tracked", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("is_tracked() missing required argument 'obj'"));
        }
        let obj = &args[0];
        // Inline scalars are never tracked.
        if matches!(
            obj,
            PyObjectRef::SmallInt(_)
                | PyObjectRef::SmallBool(_)
                | PyObjectRef::SmallFloat(_)
                | PyObjectRef::SmallStr(_)
                | PyObjectRef::None
        ) {
            return Ok(py_bool(false));
        }
        // Helper: true if this object itself would need GC tracking.
        fn is_tracked_obj(o: &PyObjectRef) -> bool {
            if matches!(
                o,
                PyObjectRef::SmallInt(_)
                    | PyObjectRef::SmallBool(_)
                    | PyObjectRef::SmallFloat(_)
                    | PyObjectRef::SmallStr(_)
                    | PyObjectRef::None
            ) {
                return false;
            }
            let borrowed = o.borrow();
            match &*borrowed {
                PyObject::Tuple(items) => {
                    // CPython: tuple is tracked iff any element is tracked.
                    items.iter().any(|el| is_tracked_obj(el))
                }
                PyObject::FrozenSet(s) => s.iter().any(|el| is_tracked_obj(el)),
                // Mutable containers are always tracked.
                PyObject::List(_)
                | PyObject::Dict(_)
                | PyObject::Set(_)
                | PyObject::Deque { .. }
                | PyObject::ByteArray(_) => true,
                PyObject::Instance { typ, dict } => {
                    // Tuple subtypes are always tracked (CPython rule).
                    if let Some(kind) = crate::object::native_base_of_type(typ) {
                        if kind == "tuple" {
                            return true;
                        }
                    }
                    // Also check via native backing's own trackability.
                    if let Some(native) = dict.get(crate::object::NATIVE_BACKING_KEY) {
                        if is_tracked_obj(native) {
                            return true;
                        }
                    }
                    let typ_name = {
                        let tr = typ.borrow();
                        if let PyObject::Type { name, .. } = &*tr {
                            name.clone()
                        } else {
                            String::new()
                        }
                    };
                    if typ_name == "object" {
                        return false;
                    }
                    // Generic instance: tracked if any attribute value is tracked
                    // or if it has a tracked native backing (already handled).
                    dict.iter().any(|(_, v)| is_tracked_obj(v))
                }
                // Immutable scalars are untracked.
                PyObject::None
                | PyObject::Bool(_)
                | PyObject::Int(_)
                | PyObject::Float(_)
                | PyObject::Str(_)
                | PyObject::Bytes(_)
                | PyObject::Complex(_, _) => false,
                _ => false,
            }
        }
        Ok(py_bool(is_tracked_obj(obj)))
    });

    // `gc.set_threshold`/`gc.get_threshold` — were missing entirely
    // (`AttributeError`). This interpreter's cycle collector (`cycle_gc.rs`)
    // uses its own fixed collection-threshold constant, not the real
    // generational gen0/gen1/gen2 thresholds CPython tunes here — so this
    // doesn't actually change collection behavior, but it stores whatever
    // was set (defaulting to CPython's own real default, `(700, 10, 10)`)
    // so `get_threshold()` reflects it, which is enough for real code that
    // just wants to read back what it set (or merely calls `set_threshold`
    // to reduce GC pauses, as `test_weakref.py`/`test_weakset.py` do, never
    // asserting on the actual collection cadence).
    thread_local! {
        static GC_THRESHOLDS: std::cell::Cell<(i64, i64, i64)> = const { std::cell::Cell::new((700, 10, 10)) };
    }
    gc_func!("set_threshold", |args| {
        let g0 = args.first().and_then(|a| a.as_i64()).unwrap_or(700);
        let g1 = args.get(1).and_then(|a| a.as_i64()).unwrap_or(10);
        let g2 = args.get(2).and_then(|a| a.as_i64()).unwrap_or(10);
        GC_THRESHOLDS.with(|c| c.set((g0, g1, g2)));
        Ok(py_none())
    });
    gc_func!("get_threshold", |_| {
        let (g0, g1, g2) = GC_THRESHOLDS.with(|c| c.get());
        Ok(py_tuple(vec![py_int(g0), py_int(g1), py_int(g2)]))
    });

    // `gc.get_debug`/`set_debug`/the `DEBUG_*` flag constants — were
    // missing entirely (`AttributeError`), breaking `test_gc.py`'s own
    // `setUpModule` (which unconditionally calls `gc.get_debug()` to save
    // and later restore the debug flags around every test). This
    // interpreter's cycle collector has no debug-tracing output to gate,
    // so this just stores whatever was set (defaulting to `0`, matching
    // real CPython) without acting on it.
    thread_local! {
        static GC_DEBUG_FLAGS: std::cell::Cell<i64> = const { std::cell::Cell::new(0) };
    }
    gc_func!("get_debug", |_| {
        Ok(py_int(GC_DEBUG_FLAGS.with(|c| c.get())))
    });
    gc_func!("set_debug", |args| {
        let flags = args.first().and_then(|a| a.as_i64()).unwrap_or(0);
        GC_DEBUG_FLAGS.with(|c| c.set(flags));
        Ok(py_none())
    });
    d.insert_str("DEBUG_STATS", py_int(1));
    d.insert_str("DEBUG_COLLECTABLE", py_int(2));
    d.insert_str("DEBUG_UNCOLLECTABLE", py_int(4));
    d.insert_str("DEBUG_SAVEALL", py_int(32));
    d.insert_str("DEBUG_LEAK", py_int(38));

    d
}
