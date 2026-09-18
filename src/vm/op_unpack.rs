use crate::bytecode::Opcode;
use crate::object::*;
use crate::vm::VirtualMachine;
use num_traits::ToPrimitive;

/// Indexed starred-unpack for sized subscriptables (CPython unpacks via
/// `len` + indexed gets, never full iteration): `Some(items)` in natural
/// order, or `None` when unsized (caller falls back to iterator drain).
/// With `discard_star` only before/after elements are fetched (the star
/// list is dropped anyway). Getitem errors propagate (like the drain
/// path's non-StopIteration errors); a short `__len__` yields a short vec
/// so the shared tail raises the usual "not enough values".
fn indexed_unpack_items(
    seq: &PyObjectRef,
    before: usize,
    after: usize,
    discard_star: bool,
) -> PyResult<Option<Vec<PyObjectRef>>> {
    let len_val = match crate::object::builtin_len(std::slice::from_ref(seq)) {
        Ok(v) => v,
        Err(_) => return Ok(None),
    };
    let n: usize = match &*len_val.borrow() {
        PyObject::Int(n) => match n.to_usize() {
            Some(n) => n,
            None => return Ok(None),
        },
        _ => return Ok(None),
    };
    let mut items = Vec::new();
    if n < before + after {
        for i in 0..n {
            items.push(crate::object::py_getitem(seq, &py_int(i as i64))?);
        }
        return Ok(Some(items));
    }
    for i in 0..before {
        items.push(crate::object::py_getitem(seq, &py_int(i as i64))?);
    }
    if discard_star {
        for i in (n - after)..n {
            items.push(crate::object::py_getitem(seq, &py_int(i as i64))?);
        }
    } else {
        for i in before..n {
            items.push(crate::object::py_getitem(seq, &py_int(i as i64))?);
        }
    }
    Ok(Some(items))
}

impl VirtualMachine {
    pub(crate) fn handle_unpack(&mut self, fi: usize, op: Opcode, arg: u32) -> PyResult<bool> {
        match op {
            Opcode::UNPACK_SEQUENCE => {
                let count = arg as usize;
                let seq = self.frames[fi].pop()?;
                // Real CPython reports an accurate "got N" count for the
                // "too many" case only when N is known cheaply/reliably up
                // front (list/tuple/dict all have an O(1) exact length) —
                // for a generic iterator it deliberately does NOT report a
                // count (see the `known_exact_count = false` branch below),
                // since computing one would mean fully draining the
                // iterable just to report a number nobody asked for.
                let (list_items, known_exact_count) = {
                    let obj = seq.borrow();
                    match &*obj {
                        PyObject::List(v) | PyObject::Tuple(v) => (Some(v.clone()), true),
                        PyObject::Dict(d) => (
                            Some(d.items().iter().map(|(k, _)| k.clone()).collect()),
                            true,
                        ),
                        _ => (None, false),
                    }
                };
                let items = match list_items {
                    Some(v) => v,
                    None => {
                        let iterator =
                            crate::object::builtin_iter(&[seq.clone()]).map_err(|_| {
                                PyError::type_error(format!(
                                    "cannot unpack non-iterable {} object",
                                    seq.borrow().type_name()
                                ))
                            })?;
                        // Stop as soon as we have proof of "too many" (count+1
                        // items) instead of draining the whole iterator —
                        // real CPython's UNPACK_SEQUENCE only ever calls
                        // `next()` count+1 times. A `__getitem__`-based
                        // sequence whose index never raises IndexError (e.g.
                        // test_unpack.py's `LengthTooLong`/`BadLength`) would
                        // otherwise loop forever here.
                        let mut v = Vec::new();
                        while v.len() <= count {
                            match crate::object::builtin_next(&[iterator.clone()]) {
                                Ok(val) => v.push(val),
                                Err(e) if crate::object::is_stop_iteration_error(&e) => break,
                                Err(e) => return Err(e),
                            }
                        }
                        v
                    }
                };
                if items.len() != count {
                    return Err(PyError::value_error(if items.len() < count {
                        format!(
                            "not enough values to unpack (expected {}, got {})",
                            count,
                            items.len()
                        )
                    } else if known_exact_count {
                        format!(
                            "too many values to unpack (expected {}, got {})",
                            count,
                            items.len()
                        )
                    } else {
                        format!("too many values to unpack (expected {})", count)
                    }));
                }
                for item in items.into_iter().rev() {
                    self.frames[fi].push(item);
                }
            }

            Opcode::UNPACK_EX => {
                let before = ((arg >> 8) & 0xFF) as usize;
                let after = (arg & 0xFF) as usize;
                // Bit 16 (set by the match-statement star path): the star
                // target is a wildcard (`[*_]` with siblings) — its list
                // is dropped, so CPython never materializes it (a 2**30
                // range must not build a billion-element list).
                let discard_star = (arg >> 16) & 1 == 1;
                let seq = self.frames[fi].pop()?;
                let list_items = {
                    let obj = seq.borrow();
                    match &*obj {
                        PyObject::List(v) | PyObject::Tuple(v) => Some(v.clone()),
                        _ => None,
                    }
                };
                let items = match list_items {
                    Some(v) => v,
                    None => match indexed_unpack_items(&seq, before, after, discard_star)? {
                        Some(v) => v,
                        None => {
                            let iterator =
                                crate::object::builtin_iter(&[seq.clone()]).map_err(|_| {
                                    PyError::type_error(format!(
                                        "cannot unpack non-iterable {} object",
                                        seq.borrow().type_name()
                                    ))
                                })?;
                            let mut v = Vec::new();
                            loop {
                                match crate::object::builtin_next(&[iterator.clone()]) {
                                    Ok(val) => v.push(val),
                                    Err(e) if crate::object::is_stop_iteration_error(&e) => break,
                                    Err(e) => return Err(e),
                                }
                            }
                            v
                        }
                    },
                };
                if items.len() < before + after {
                    return Err(PyError::value_error(format!(
                        "not enough values to unpack (expected at least {}, got {})",
                        before + after,
                        items.len()
                    )));
                }
                let n = items.len();
                // CPython's _PyEval_UnpackIterableStackRef uses *--sp
                // (pre-decrement), so items must be pushed onto the stack
                // in reverse order: after-elements right-to-left, then the
                // starred list, then before-elements right-to-left.

                // 1) Push after-elements in reverse order so that STORE
                //    operations (which pop from TOS) assign them left-to-right.
                for i in ((n - after)..n).rev() {
                    self.frames[fi].push(items[i].clone());
                }
                // 2) Push the starred list (or an empty placeholder when
                //    the star target is a wildcard — dropped by the case
                //    arm's POP, never materialized).
                if discard_star {
                    self.frames[fi].push(py_list(Vec::new()));
                } else {
                    let star_count = n - before - after;
                    let mut star_items: Vec<PyObjectRef> = Vec::new();
                    for i in before..(before + star_count) {
                        star_items.push(items[i].clone());
                    }
                    self.frames[fi].push(py_list(star_items));
                }
                // 3) Push before-elements (reversed) — these end up on
                //    bottom so the leftmost STORE pops them first.
                for i in (0..before).rev() {
                    self.frames[fi].push(items[i].clone());
                }
            }

            _ => return Ok(false),
        }
        Ok(true)
    }
}
