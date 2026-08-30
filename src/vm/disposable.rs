use crate::object::PyObjectRef;
use crate::vm::VirtualMachine;
use std::cell::RefCell;

thread_local! {
    pub(crate) static DISPOSABLE_VM_POOL: RefCell<Vec<VirtualMachine>> = const { RefCell::new(Vec::new()) };
}

impl VirtualMachine {
    /// Reset per-use mutable scratch state; returns self for chaining.
    pub(crate) fn reset_disposable_state(mut self) -> VirtualMachine {
        self.reset_disposable_state_ref();
        self
    }
    pub(crate) fn reset_disposable_state_ref(&mut self) {
        self.frames.clear();
        self.last_error_line = None;
        self.last_error_file = None;
        self.last_traceback.clear();
        self.exc_type = None;
        self.exc_value = None;
        self.exc_traceback = None;
        self.exc_context_stack.clear();
        self.propagating_exc = None;
        self.exec_globals_cache.clear();
    }

    /// Take a disposable VM from the thread-local pool (or build one).
    /// Generator/coroutine resumes used to call `VirtualMachine::new()`
    /// per resume (~40us each, measured); pooled reuse drops that to
    /// near-frame-cost. The VM's mutable scratch state is reset here; the
    /// shared stdlib (builtins/modules via Rc) is untouched by design.
    pub(crate) fn take_disposable() -> VirtualMachine {
        let vm = DISPOSABLE_VM_POOL
            .with(|p| p.borrow_mut().pop())
            .unwrap_or_else(VirtualMachine::new);
        vm.reset_disposable_state()
    }

    /// Return a disposable VM to the pool after scrubbing per-use state.
    pub(crate) fn release_disposable(vm: VirtualMachine) {
        let vm = vm.reset_disposable_state();
        DISPOSABLE_VM_POOL.with(|p| {
            let mut v = p.borrow_mut();
            v.push(vm);
            while v.len() > 8 {
                v.pop();
            }
        });
    }
}
