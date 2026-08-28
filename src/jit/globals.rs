use crate::interner::StrId;
use crate::object::PyObjectRef;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

thread_local! {
    pub(crate) static CURRENT_JIT_GLOBALS: RefCell<Option<Rc<RefCell<HashMap<StrId, PyObjectRef>>>>> =
        RefCell::new(None);
}

/// Set the JIT-executing function's globals for the current thread (called
/// by the VM around each JIT invocation). Returns a guard that restores the
/// previous value on drop.
pub struct JitGlobalsGuard;
impl Drop for JitGlobalsGuard {
    fn drop(&mut self) {
        CURRENT_JIT_GLOBALS.with(|g| *g.borrow_mut() = None);
    }
}
pub fn set_jit_globals(g: Rc<RefCell<HashMap<StrId, PyObjectRef>>>) -> JitGlobalsGuard {
    CURRENT_JIT_GLOBALS.with(|c| *c.borrow_mut() = Some(g));
    JitGlobalsGuard
}
