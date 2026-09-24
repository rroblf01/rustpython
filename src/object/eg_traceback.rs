// Per-instance `__traceback__` storage for `ExceptionGroup`.
//
// `PyObject::Exception` carries `traceback`/`context`/`cause` as real
// variant fields, but `PyObject::ExceptionGroup` has only `typ`/`args`/
// `exceptions` — no slot for the traceback chain the VM builds while an
// exception unwinds (`vm/except.rs::prepend_traceback`). Without this table
// every `ExceptionGroup.__traceback__` read returned `None`, so
// `test_exception_group.py`'s nested-traceback assertions blew up with
// `'NoneType' object has no attribute 'tb_lineno'`.
//
// Entries are keyed by object id and carry the group itself, so a recycled
// address can never hand a stale chain to a different group: the stored
// owner must still be the SAME allocation (pointer identity) to match. The
// table is append-only with a cap — a raised group is a short-lived object,
// and holding it past its death would be a leak; dropping the oldest entries
// bounds that to the cap.
use crate::object::{PyObject, PyObjectRef};
use std::cell::RefCell;

const MAX_ENTRIES: usize = 4096;

thread_local! {
    static TRACEBACKS: RefCell<Vec<(usize, PyObjectRef, PyObjectRef)>> =
        const { RefCell::new(Vec::new()) };
}

fn same_allocation(a: &PyObjectRef, b: &PyObjectRef) -> bool {
    match (&*a, &*b) {
        (PyObjectRef::Mut(x), PyObjectRef::Mut(y)) => std::rc::Rc::ptr_eq(x, y),
        (PyObjectRef::Imm(x), PyObjectRef::Imm(y)) => std::rc::Rc::ptr_eq(x, y),
        _ => false,
    }
}

/// The traceback chain recorded for `group`, if any.
pub(crate) fn get(group: &PyObjectRef) -> Option<PyObjectRef> {
    let id = group.get_id();
    TRACEBACKS.with(|t| {
        t.borrow()
            .iter()
            .rev()
            .find(|(entry_id, owner, _)| *entry_id == id && same_allocation(owner, group))
            .map(|(_, _, tb)| tb.clone())
    })
}

/// Record `tb` as `group`'s traceback (replacing any previous chain).
pub(crate) fn set(group: &PyObjectRef, tb: PyObjectRef) {
    let id = group.get_id();
    TRACEBACKS.with(|t| {
        let mut entries = t.borrow_mut();
        if entries.len() >= MAX_ENTRIES {
            entries.remove(0);
        }
        entries.push((id, group.clone(), tb));
    });
}
