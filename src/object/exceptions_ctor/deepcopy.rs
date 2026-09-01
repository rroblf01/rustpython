// Split from src/object/exceptions_ctor.rs — deepcopy helper.
use super::*;
use crate::object::*;

/// Helper: deep-copy a single object with memo support
pub fn deepcopy_one(obj: &PyObjectRef, memo: &PyObjectRef) -> Result<PyObjectRef, PyError> {
    // Check memo first using identity
    if let PyObject::Dict(memo_dict) = &*memo.borrow() {
        if let Some(cached) = memo_dict.get_by_identity(obj) {
            return Ok(cached);
        }
    }
    // Uses `set_by_identity` (bypasses `.hash()`) — NOT the ordinary
    // `set()`, which would call `key.hash()` and get `Err("unhashable
    // type")` for exactly the container types (dict/list/set) most likely
    // to need cycle protection here, silently failing to store anything.
    fn remember(memo: &PyObjectRef, orig: &PyObjectRef, copy: &PyObjectRef) {
        if let PyObject::Dict(memo_dict) = &mut *memo.borrow_mut() {
            memo_dict.set_by_identity(orig.clone(), copy.clone());
        }
    }
    // List/Dict are MUTABLE, so a self- or mutually-referential structure
    // (`d = {}; d['self'] = d`) is directly constructible in real Python —
    // deep-copying one must therefore create the new (still-empty)
    // container and register it in `memo` BEFORE recursing into its
    // children, so a child that refers back to the original finds the
    // (partially-built) copy already memoized instead of recursing forever.
    // The previous version only called `remember` AFTER fully copying all
    // children — for a self-referential dict/list, the recursive call for
    // the self-reference would run before its own entry ever got memoized,
    // recursing without end and overflowing the native stack (confirmed via
    // CPython's own `test_copy.py::test_deepcopy_reflexive_dict`).
    match &*obj.borrow() {
        PyObject::Int(_)
        | PyObject::Float(_)
        | PyObject::Str(_)
        | PyObject::Bool(_)
        | PyObject::None
        | PyObject::Bytes(_) => Ok(obj.clone()),
        PyObject::List(_) => {
            let new_list = py_list(Vec::new());
            remember(memo, obj, &new_list);
            let items = if let PyObject::List(items) = &*obj.borrow() {
                items.clone()
            } else {
                unreachable!()
            };
            let mut new_items = Vec::with_capacity(items.len());
            for item in &items {
                new_items.push(deepcopy_one(item, memo)?);
            }
            if let PyObject::List(nl) = &mut *new_list.borrow_mut() {
                *nl = new_items;
            }
            Ok(new_list)
        }
        PyObject::Deque { .. } => {
            let new_deque = py_deque(std::collections::VecDeque::new(), None);
            remember(memo, obj, &new_deque);
            let (items, maxlen) = if let PyObject::Deque { data, maxlen } = &*obj.borrow() {
                (data.iter().cloned().collect::<Vec<_>>(), *maxlen)
            } else {
                unreachable!()
            };
            let mut new_data = std::collections::VecDeque::new();
            for item in &items {
                new_data.push_back(deepcopy_one(item, memo)?);
            }
            if let PyObject::Deque { data, maxlen: ml } = &mut *new_deque.borrow_mut() {
                *data = new_data;
                *ml = maxlen;
            }
            Ok(new_deque)
        }
        PyObject::Dict(_) => {
            let new_dict = PyObjectRef::new(PyObject::Dict(Box::new(PyDict::new())));
            remember(memo, obj, &new_dict);
            let items = if let PyObject::Dict(d) = &*obj.borrow() {
                d.items()
            } else {
                unreachable!()
            };
            for (k, v) in items {
                let new_k = deepcopy_one(&k, memo)?;
                let new_v = deepcopy_one(&v, memo)?;
                if let PyObject::Dict(nd) = &mut *new_dict.borrow_mut() {
                    let _ = nd.set(new_k, new_v);
                }
            }
            Ok(new_dict)
        }
        // Tuples are immutable, so a PURE tuple-only cycle can never exist
        // in real Python (a tuple can only reference already-fully-built
        // objects) — no placeholder-first trick needed here, just the
        // ordinary "build children, then memoize the final result" shape
        // (still useful for diamond references: the same tuple appearing
        // twice in one structure should deep-copy to the same new object).
        PyObject::Tuple(items) => {
            let items = items.clone();
            let mut new_items = Vec::with_capacity(items.len());
            for item in &items {
                new_items.push(deepcopy_one(item, memo)?);
            }
            let result = PyObjectRef::imm(PyObject::Tuple(new_items));
            remember(memo, obj, &result);
            Ok(result)
        }
        // A `slice`'s `start`/`stop`/`step` can be arbitrary (mutable)
        // objects, not just ints (see the `.start`/`.stop`/`.step`
        // attribute-getter's own doc comment in `attrs.rs`) — was falling
        // to the generic `_` fallback below, which has no
        // `native_backing_of`/`__deepcopy__` for a plain `Slice` and so
        // just cloned the `Rc`, returning the SAME object. Real Python
        // deep-copies each of the three fields independently (confirmed:
        // `test_slice.py::test_deepcopy`'s "corner case for mutable
        // indices", `slice([1,2],[3,4],[5,6])`, asserts the copy `is not`
        // the original AND each field `is not` its original counterpart).
        PyObject::Slice { start, stop, step } => {
            let (start, stop, step) = (start.clone(), stop.clone(), step.clone());
            let new_start = deepcopy_one(&start, memo)?;
            let new_stop = deepcopy_one(&stop, memo)?;
            let new_step = deepcopy_one(&step, memo)?;
            let result = PyObjectRef::imm(PyObject::Slice {
                start: new_start,
                stop: new_stop,
                step: new_step,
            });
            remember(memo, obj, &result);
            Ok(result)
        }
        _ => {
            // Custom `__deepcopy__` takes priority (matching real Python's
            // `copy.deepcopy` protocol) — without this, an Instance nested
            // inside a list/dict/tuple being deep-copied always got a bare
            // shallow `.clone()` instead of ever invoking its own
            // `__deepcopy__`.
            if let Ok(dc_method) = obj.borrow().get_attribute("__deepcopy__") {
                // `call_function_disposable` (a user `__deepcopy__` is a
                // Python Function; the bare `call_function` here only handles
                // BuiltinFunction/Closure).
                let result = crate::object::call_function_disposable(
                    &dc_method,
                    vec![obj.clone(), memo.clone()],
                    vec![],
                )?;
                remember(memo, obj, &result);
                return Ok(result);
            }
            // Same native-base-subclass gap as `copy.copy`'s own fallback
            // (`misc.rs`) — a class transparently subclassing a native
            // container with no `__deepcopy__` override fell straight to
            // `obj.clone()` (an `Rc` clone, the SAME object), instead of
            // recursively deep-copying its actual contents. Deep-copy the
            // native backing's elements (not just a shallow copy of the
            // top-level container, unlike `copy.copy`) and wrap the result
            // in a NEW `Instance` of the same class.
            if let Some(native) = native_backing_of(obj) {
                let placeholder = PyObjectRef::new(PyObject::None);
                remember(memo, obj, &placeholder);
                let new_native = deepcopy_one(&native, memo)?;
                let (typ, dict) = if let PyObject::Instance { typ, dict } = &*obj.borrow() {
                    (typ.clone(), dict.clone())
                } else {
                    unreachable!()
                };
                let mut new_dict = dict;
                new_dict.insert(NATIVE_BACKING_KEY.to_string(), new_native);
                let result = PyObjectRef::new(PyObject::Instance {
                    typ,
                    dict: new_dict,
                });
                if let PyObject::Dict(memo_dict) = &mut *memo.borrow_mut() {
                    memo_dict.set_by_identity(obj.clone(), result.clone());
                }
                return Ok(result);
            }
            let result = obj.clone();
            remember(memo, obj, &result);
            Ok(result)
        }
    }
}
