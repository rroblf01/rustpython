use crate::ast::Stmt;
use std::collections::HashSet;

#[derive(Clone)]
pub enum PendingCleanup {
    With(bool), // is_async
    Finally(Vec<Stmt>),
    // Marks "we're compiling an `except` handler's body, whose entry point
    // (`PUSH_EXC_INFO`) pushed the active exception onto the stack" — a
    // `return`/`break`/`continue` from inside that body must `POP_EXCEPT`
    // that pushed value before jumping out, exactly like the handler's own
    // normal fall-through path already does. Without this, `return` from
    // inside `except X: return val` (an extremely common pattern —
    // `import_fresh_module`'s own `except ImportError: return None`) left
    // the exception-info value permanently on the stack: harmless by
    // itself, but any ENCLOSING `with` block's return-cleanup inlining
    // (`PendingCleanup::With`, above) then swaps/dups/calls `__exit__` on
    // whatever's now on top of the stack — the stray exception object,
    // not the real context manager — surfacing as `AttributeError:
    // 'ImportError' object has no attribute '__exit__'` several statements
    // away from the actual bug.
    PopExcept,
}

pub struct LoopInfo {
    pub start_label: usize,
    pub end_label: usize,
    // A `break`/`continue` must run only the pending cleanups (with/except)
    // registered WITHIN the loop body — cleanups registered OUTSIDE the
    // loop (e.g. a `with` wrapping the whole loop: `with cm: for x: break`)
    // run naturally at their own scope's end, and running them inline at
    // the break would corrupt the stack (the outer loop's iterator sits
    // above the with-manager). `cleanup_start` snapshots pending_cleanup's
    // length at loop start.
    pub cleanup_start: usize,
    // `for`/`async for` loops keep their iterator object sitting on the
    // stack for the loop's whole duration (FOR_ITER peeks it each pass;
    // END_FOR pops it once on natural exhaustion, right before
    // `end_label`). A `break` jumps straight to `end_label`, skipping that
    // END_FOR — so without popping it here too, every `break` inside any
    // `for` loop permanently leaked one stack slot into the enclosing
    // frame, corrupting everything after it (confirmed: a `break` in a
    // `for` loop nested inside another `for`/`while` loop silently
    // desynced the outer loop's own iteration, either skipping values or
    // looping forever). `while` loops push nothing extra, so this stays
    // `false` there and `break` needs no compensating pop.
    pub is_for: bool,
}

pub struct ScopeInfo {
    pub scope: ScopeType,
    pub global_names: HashSet<String>,
    pub nonlocal_names: HashSet<String>,
}

#[derive(Clone, PartialEq, Debug)]
pub enum ScopeType {
    Module,
    Function,
    ClassBody,
}
