//! The "rare event" counters CPython's adaptive interpreter keeps
//! (`_testinternalcapi.get_rare_event_counters`).
//!
//! CPython bumps one when it has to *give up* specialising something: setting
//! an instance's `__class__`, assigning a class's `__bases__`, mutating the
//! builtins dict or a function's `__code__`, installing an eval-frame hook.
//! There is no adaptive interpreter here, so every counter stays at zero —
//! but `test_optimizer` reads them and asserts they go UP by one for each of
//! those operations, so the operations have to be counted, not just the
//! reading.
//!
//! Exposed to Python through the native `_rare_events` module, which
//! `Lib/_testinternalcapi.py` wraps.

use std::sync::atomic::{AtomicU64, Ordering};

pub const SET_CLASS: usize = 0;
pub const SET_BASES: usize = 1;
pub const SET_EVAL_FRAME_FUNC: usize = 2;
pub const BUILTIN_DICT: usize = 3;
pub const FUNC_MODIFICATION: usize = 4;

const NAMES: [&str; 5] = [
    "set_class",
    "set_bases",
    "set_eval_frame_func",
    "builtin_dict",
    "func_modification",
];

static COUNTERS: [AtomicU64; NAMES.len()] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];

/// Count one rare event. Cheap enough to sit on the assignment paths.
#[inline]
pub fn bump(which: usize) {
    if which < NAMES.len() {
        COUNTERS[which].fetch_add(1, Ordering::Relaxed);
    }
}

pub fn get(which: usize) -> u64 {
    if which < NAMES.len() {
        COUNTERS[which].load(Ordering::Relaxed)
    } else {
        0
    }
}

pub fn reset() {
    for c in COUNTERS.iter() {
        c.store(0, Ordering::Relaxed);
    }
}

pub fn snapshot() -> Vec<(&'static str, u64)> {
    NAMES
        .iter()
        .enumerate()
        .map(|(i, n)| (*n, get(i)))
        .collect()
}
