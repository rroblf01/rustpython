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
    /// shared stdlib (builtins/modules via Rc) is untouched by design —
    /// EXCEPT for `collections.abc`/`_collections_abc`, re-synced below.
    ///
    /// Why the exception: a pooled instance's `self.modules` is frozen at
    /// whatever it held when it was first built (or last released) and is
    /// never refreshed again — fine for ordinary stdlib modules, which are
    /// already present at VM-construction time and only ever get mutated
    /// in place (visible through the shared `Rc` regardless of which
    /// instance holds the reference). `collections.abc` is different: it's
    /// installed via a one-time, reentrancy-guarded self-import
    /// (`install_collections_abc_alias`, see its own doc comment), and any
    /// disposable VM built WHILE that one-time import is still in flight
    /// (its `abc.ABCMeta`/`_weakrefset` machinery drives plenty of
    /// generators, each of which can itself request a disposable VM) hits
    /// the reentrancy guard and skips installing the alias for itself —
    /// permanently, if that particular instance then lands in this pool.
    /// Confirmed via a standalone repro: a script doing `import dbm` then,
    /// inside a generator, `__import__("dbm.dumb", fromlist=["open"])`
    /// (exactly what `dbm/__init__.py`'s own dispatch helpers do) got a
    /// `dbm.dumb` whose top-level `import collections.abc` statement raised
    /// `ModuleNotFoundError` — collapsed by the generic `except ImportError`
    /// around it into a silently-skipped import, leaving a partially
    /// executed, `.open`-less module cached forever after. Re-running the
    /// installer here is effectively free once the module is cached (an
    /// early `self.modules.contains_key(...)` check short-circuits it), so
    /// it fixes every pooled instance without reintroducing the original
    /// per-resume `VirtualMachine::new()` cost this pool exists to avoid.
    pub(crate) fn take_disposable() -> VirtualMachine {
        let mut vm = DISPOSABLE_VM_POOL
            .with(|p| p.borrow_mut().pop())
            .unwrap_or_else(VirtualMachine::new);
        // Re-sync a STALE pooled snapshot — one that is MISSING modules the
        // real VM imported since (the startup `encodings` import is the case
        // that matters). Comparing the module COUNT is O(1) and strictly
        // one-sided (`>` not `!=`): re-cloning the ~113-entry map on every
        // take (as an unconditional refresh did) slowed every generator
        // resume enough to turn test_decimal (14s -> 151s) and
        // test_threading into 180s timeouts, and a VM that merely imported
        // something EXTRA is not stale, so `!=` would have paid that cost
        // again for the rest of the process.
        let cached_len = crate::vm::cached_module_count();
        if cached_len > vm.modules.len() {
            crate::vm::refresh_modules_from_state_cache(&mut vm);
        }
        // NOTE: the module map is deliberately NOT re-synced here. A
        // POOLED instance's `modules` is a snapshot frozen when it was first
        // built, so anything imported into the real VM afterwards (notably
        // the startup `encodings` import, whose `codecs.register(
        // search_function)` is what makes `codecs.lookup()` find the real
        // `encodings/*` codecs) was invisible to it — and its first
        // `import encodings.<x>` RE-EXECUTED `Lib/encodings/__init__.py` and
        // `Lib/codecs.py`, installing a SECOND `codecs` module object into
        // the shared `sys.modules` dict. Everything bound to the first one
        // then compared against a different `codecs.CodecInfo` class and
        // every `getregentry()` result took the "not a CodecInfo" rebuild
        // path, silently dropping each codec's `incrementalencoder` /
        // `incrementaldecoder` (`codecs.getincrementaldecoder('utf-8-sig')`
        // then raised `LookupError`). The fix lives in the IMPORT path
        // instead — `import_module_from_file_inner` falls back to the
        // process-wide module cache on a miss — because doing the sync here
        // cost a ~113-entry `HashMap` clone on EVERY disposable VM
        // (i.e. on every generator resume), which turned
        // `test_decimal`/`test_threading` into 180s timeouts.
        vm.install_collections_abc_alias();
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
