//! Cycle-collecting garbage collector for the real, always-on object model
//! (`PyObjectRef::Mut`/`Imm`, both `Rc<RefCell<PyObject>>`) — NOT the
//! separate, unwired, `gc` Cargo-feature-gated tracing heap in `gc.rs`
//! (which replaces the object model entirely and isn't used by default).
//!
//! Plain reference counting (what this interpreter already uses for every
//! object) can never free a REFERENCE CYCLE — two objects that point at
//! each other (or a longer chain back to itself) keep each other's
//! refcount above zero forever, even once nothing outside the cycle
//! references either of them. Confirmed to be a REAL, severe leak: 200,000
//! short-lived `parent`/`child` cross-referencing pairs (created and then
//! immediately dropped by the caller) balloon memory to 367MB here, versus
//! 15MB in real CPython (which has had a supplementary cycle collector
//! since Python 2.0).
//!
//! This module implements the same algorithm CPython's own `gc` module
//! uses conceptually — "trial deletion": for every tracked object, subtract
//! one from its real strong refcount for each reference found coming from
//! ANOTHER tracked object. Whatever remains positive must be referenced
//! from OUTSIDE the tracked set (a VM frame's local variable, a global, the
//! builtins table, ...) — i.e. a genuine root, with no need to enumerate
//! those roots explicitly. A breadth-first walk from every such root marks
//! everything transitively reachable as live; anything tracked but never
//! marked is unreachable garbage — even if it forms a cycle — and gets its
//! internal references cleared so the ordinary `Rc` drop glue can actually
//! deallocate it.
//!
//! Only "container" object kinds that can plausibly hold other
//! `PyObjectRef`s are tracked (matching CPython's own `gc`, which likewise
//! never tracks plain ints/strings/etc.) — see `is_trackable`.

use crate::object::{PyDict, PyObject, PyObjectRef, PySet};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};

thread_local! {
    static REGISTRY: RefCell<Vec<Weak<RefCell<PyObject>>>> = RefCell::new(Vec::new());
    static ALLOCS_SINCE_COLLECT: Cell<usize> = Cell::new(0);
    static ENABLED: Cell<bool> = Cell::new(true);
    static LAST_STATS: Cell<(usize, usize)> = Cell::new((0, 0)); // (tracked, collected) as of last collect()
}

/// Mirrors CPython's generation-0 default threshold (700) in spirit, tuned
/// up since every collection pass here is a full O(tracked) scan (no
/// generational promotion of long-lived objects yet — see this module's
/// own doc comment for why that matters and is worth adding later): a
/// program that legitimately keeps a large, stable set of tracked objects
/// alive (e.g. a big cache) pays that same O(tracked) cost on EVERY pass
/// regardless of how little garbage exists, so collecting less often
/// reduces total overhead more than it delays real reclamation in practice.
const COLLECTION_THRESHOLD: usize = 20_000;

/// Whether `obj`'s kind can hold other `PyObjectRef`s and therefore needs to
/// be tracked as a potential cycle participant. Kept to the classic
/// cycle-forming shapes (containers, instances, closures) rather than
/// every `Mut`/`Imm` variant — e.g. `Type`/`Module`/`Exception` are still
/// correctly traced *through* by the mark phase (see `trace_children`) when
/// reachable from something that IS tracked, they're just not tracked
/// (registered, counted, or collectible) themselves: in practice they're
/// long-lived and already kept alive by real external tables
/// (`vm.modules`, `vm.type_registry`, `vm.builtins`), so tracking them too
/// would only add bookkeeping cost without real collection benefit.
pub fn is_trackable(obj: &PyObject) -> bool {
    matches!(
        obj,
        PyObject::List(_)
            | PyObject::Tuple(_)
            | PyObject::Dict(_)
            | PyObject::Set(_)
            | PyObject::FrozenSet(_)
            | PyObject::Instance { .. }
            | PyObject::Function { .. }
            | PyObject::BoundMethod { .. }
            | PyObject::Cell { .. }
            | PyObject::Partial { .. }
    )
}

/// Register a freshly-allocated, trackable `Rc` and opportunistically
/// trigger a collection once enough tracked allocations have piled up since
/// the last one. Called from `PyObjectRef::new`/`PyObjectRef::imm` — see
/// there.
pub fn track(rc: &Rc<RefCell<PyObject>>) {
    if !ENABLED.with(Cell::get) {
        return;
    }
    REGISTRY.with(|r| r.borrow_mut().push(Rc::downgrade(rc)));
    let hit_threshold = ALLOCS_SINCE_COLLECT.with(|c| {
        let n = c.get() + 1;
        c.set(n);
        n >= COLLECTION_THRESHOLD
    });
    if hit_threshold {
        ALLOCS_SINCE_COLLECT.with(|c| c.set(0));
        collect();
    }
}

pub fn set_enabled(enabled: bool) {
    ENABLED.with(|c| c.set(enabled));
}

pub fn is_enabled() -> bool {
    ENABLED.with(Cell::get)
}

/// (currently tracked object count, objects collected by the last `collect()` call)
pub fn stats() -> (usize, usize) {
    LAST_STATS.with(Cell::get)
}

fn extract_rc(r: &PyObjectRef) -> Option<Rc<RefCell<PyObject>>> {
    match r {
        PyObjectRef::Mut(rc) | PyObjectRef::Imm(rc) => Some(rc.clone()),
        _ => None,
    }
}

fn trace_dict(d: &PyDict, out: &mut Vec<PyObjectRef>) {
    for (k, v) in d.items() {
        out.push(k);
        out.push(v);
    }
}

fn trace_set(s: &PySet, out: &mut Vec<PyObjectRef>) {
    out.extend(s.to_vec());
}

/// Every direct child `PyObjectRef` an object owns. Used for BOTH the
/// trial-deletion decrement pass (children of TRACKED objects only matter
/// there) and the reachability walk (which must pass straight through
/// non-tracked kinds too — e.g. a reachable `Type`'s `dict` might hold a
/// tracked `Function`).
fn trace_children(obj: &PyObject, out: &mut Vec<PyObjectRef>) {
    match obj {
        PyObject::List(v) | PyObject::Tuple(v) => out.extend(v.iter().cloned()),
        PyObject::Dict(d) => trace_dict(d, out),
        PyObject::Set(s) | PyObject::FrozenSet(s) => trace_set(s, out),
        PyObject::Instance { typ, dict } => {
            out.push(typ.clone());
            out.extend(dict.values().cloned());
        }
        PyObject::Function { defaults, closure, dict, .. } => {
            // Deliberately NOT tracing `globals` (the module's own
            // namespace) — it's always reachable via the module object
            // itself, and tracing it here would make every function
            // pull in everything else in its defining module as a
            // "child" for no real benefit.
            out.extend(defaults.iter().cloned());
            out.extend(closure.iter().cloned());
            out.extend(dict.values().cloned());
        }
        PyObject::BoundMethod { func, self_obj } => {
            out.push(func.clone());
            out.push(self_obj.clone());
        }
        PyObject::Cell { value: Some(v) } => out.push(v.clone()),
        PyObject::Partial { func, args } => {
            out.push(func.clone());
            out.extend(args.iter().cloned());
        }
        PyObject::Type { dict, bases, mro, .. } => {
            out.extend(dict.values().cloned());
            out.extend(bases.iter().cloned());
            out.extend(mro.iter().cloned());
        }
        PyObject::Module { dict, .. } => out.extend(dict.values().cloned()),
        PyObject::Exception { args, cause, .. } => {
            out.extend(args.iter().cloned());
            if let Some(c) = cause {
                out.push(c.clone());
            }
        }
        PyObject::ExceptionGroup { args, exceptions, .. } => {
            out.extend(args.iter().cloned());
            out.extend(exceptions.iter().cloned());
        }
        PyObject::Super { cls, obj } => {
            out.push(cls.clone());
            out.push(obj.clone());
        }
        PyObject::Property { getter, setter, deleter, .. } => {
            if let Some(g) = getter {
                out.push(g.clone());
            }
            if let Some(s) = setter {
                out.push(s.clone());
            }
            if let Some(d) = deleter {
                out.push(d.clone());
            }
        }
        PyObject::StaticMethod { func } | PyObject::ClassMethod { func } => out.push(func.clone()),
        PyObject::ZipIterator { iterators } => out.extend(iterators.iter().cloned()),
        PyObject::MapIterator { func, iterator } => {
            out.push(func.clone());
            out.push((**iterator).clone());
        }
        PyObject::FilterIterator { func, iterator } => {
            out.push(func.clone());
            out.push((**iterator).clone());
        }
        PyObject::ListIter { list: items, .. } => {
            out.extend(items.iter().cloned())
        }
        PyObject::EnumerateIter { source, .. } => out.push(source.clone()),
        PyObject::CycleIter { items, .. } => out.extend(items.iter().cloned()),
        PyObject::Slice { start, stop, step } => {
            out.push(start.clone());
            out.push(stop.clone());
            out.push(step.clone());
        }
        _ => {}
    }
}

/// Severs every outgoing reference a GARBAGE (unreachable, possibly
/// cyclic) object holds, so its former cyclic partners lose their only
/// remaining strong reference and the ordinary `Rc` drop glue can actually
/// deallocate them. Only needs to handle the kinds `is_trackable` tracks
/// (those are the only ones ever collected) — this is intentionally a
/// SUBSET of `trace_children`'s match.
fn clear_children(obj: &mut PyObject) {
    match obj {
        PyObject::List(v) | PyObject::Tuple(v) => v.clear(),
        PyObject::Dict(d) => d.clear(),
        PyObject::Set(s) | PyObject::FrozenSet(s) => s.clear(),
        PyObject::Instance { dict, .. } => dict.clear(),
        PyObject::Function { defaults, closure, dict, .. } => {
            defaults.clear();
            closure.clear();
            dict.clear();
        }
        PyObject::BoundMethod { func, self_obj } => {
            *func = crate::object::py_none();
            *self_obj = crate::object::py_none();
        }
        PyObject::Cell { value } => *value = None,
        PyObject::Partial { args, .. } => args.clear(),
        _ => {}
    }
}

/// Run one cycle-collection pass over every currently tracked object.
/// Returns how many were found to be garbage (and had their contents
/// cleared) this pass. Safe to call at any point with no other borrows of
/// tracked objects active (matches every other `RefCell` use in this
/// codebase — a live conflicting borrow panics rather than corrupting
/// state).
pub fn collect() -> usize {
    // Snapshot every still-alive tracked object, dropping dead registry
    // entries (already collected by refcounting alone) as we go. Holding a
    // real `Rc` per entry for the rest of this function is deliberate — it
    // guarantees nothing we're about to inspect can be deallocated out from
    // under us mid-pass, at the cost of inflating every strong count by
    // exactly 1 (corrected for explicitly below).
    let live_rcs: Vec<Rc<RefCell<PyObject>>> = REGISTRY.with(|r| {
        let mut reg = r.borrow_mut();
        let mut alive = Vec::with_capacity(reg.len());
        reg.retain(|w| match w.upgrade() {
            Some(rc) => {
                alive.push(rc);
                true
            }
            None => false,
        });
        alive
    });

    if live_rcs.is_empty() {
        LAST_STATS.with(|c| c.set((0, 0)));
        return 0;
    }

    let mut index_of: HashMap<*const RefCell<PyObject>, usize> = HashMap::with_capacity(live_rcs.len());
    for (i, rc) in live_rcs.iter().enumerate() {
        index_of.insert(Rc::as_ptr(rc), i);
    }

    // `- 1`: undo the inflation from this function's own `live_rcs` clone.
    let mut gc_refs: Vec<isize> = live_rcs.iter().map(|rc| Rc::strong_count(rc) as isize - 1).collect();

    // Trial deletion: subtract one for every reference found coming from
    // another TRACKED object (cycle-internal references don't count as
    // "external" reachability).
    let mut scratch = Vec::new();
    for rc in &live_rcs {
        scratch.clear();
        // Use try_borrow() to avoid panicking on objects that are currently
        // mutably borrowed by running code. If we can't borrow, skip this
        // object — the cycle might not be fully collected this cycle, but
        // it will be revisited on a future GC run.
        let borrowed = match rc.try_borrow() {
            Ok(b) => b,
            Err(_) => continue,
        };
        trace_children(&borrowed, &mut scratch);
        for child in &scratch {
            if let Some(child_rc) = extract_rc(child) {
                if let Some(&j) = index_of.get(&Rc::as_ptr(&child_rc)) {
                    gc_refs[j] -= 1;
                }
            }
        }
    }

    // Anything still positive has a real external referrer (a VM frame
    // local, a global, `vm.modules`/`vm.builtins`, ...) — no need to
    // enumerate those roots explicitly, the refcount arithmetic already
    // finds them. BFS from each one, marking everything transitively
    // reachable as live — walking through ANY object (tracked or not), so
    // e.g. a reachable but untracked `Type`'s dict can still lead to a
    // tracked `Function`.
    let mut live = vec![false; live_rcs.len()];
    let mut visited: HashSet<*const RefCell<PyObject>> = HashSet::new();
    let mut stack: Vec<Rc<RefCell<PyObject>>> = Vec::new();
    let mut seed_count = 0;
    for (i, rc) in live_rcs.iter().enumerate() {
        if gc_refs[i] > 0 {
            seed_count += 1;
            live[i] = true;
            if visited.insert(Rc::as_ptr(rc)) {
                stack.push(rc.clone());
            }
        }
    }
    if std::env::var("RPY_DEBUG_GC").is_ok() {
        eprintln!("cycle_gc: seed_count={} (gc_refs>0 before BFS)", seed_count);
    }
    while let Some(rc) = stack.pop() {
        scratch.clear();
        // Use try_borrow() (not borrow()), matching the trial-deletion loop
        // above — a collection can trigger while some in-progress operation
        // (a `set.add()`/`dict.__setitem__()` call in the middle of
        // computing a key's hash, an iterator mid-advance, ...) already
        // holds a real borrow on an object this BFS also happens to reach
        // (real trigger: CPython's own `test_set.py`/`test_iter.py`,
        // wherever the 20,000-allocation auto-collect threshold happened to
        // land mid-operation). Skipping an unborrowable object here just
        // means whatever it uniquely reaches isn't marked live THIS pass —
        // safe, since anything genuinely still needed has its own live
        // external referrer keeping its own refcount up, and a future GC
        // pass gets another chance once the borrow is released.
        if let Ok(borrowed) = rc.try_borrow() {
            trace_children(&borrowed, &mut scratch);
        }
        for child in scratch.drain(..) {
            if let Some(child_rc) = extract_rc(&child) {
                let ptr = Rc::as_ptr(&child_rc);
                if let Some(&j) = index_of.get(&ptr) {
                    live[j] = true;
                }
                if visited.insert(ptr) {
                    stack.push(child_rc);
                }
            }
        }
    }

    // Anything tracked but never marked live is unreachable — even though
    // plain refcounting alone would have kept it around forever (that's
    // the whole point). Sever its outgoing references so the graph can
    // actually collapse.
    let mut collected = 0;
    for (i, rc) in live_rcs.iter().enumerate() {
        if !live[i] {
            clear_children(&mut rc.borrow_mut());
            collected += 1;
        }
    }
    LAST_STATS.with(|c| c.set((live_rcs.len(), collected)));
    if std::env::var("RPY_DEBUG_GC").is_ok() {
        eprintln!("cycle_gc: tracked={} collected={}", live_rcs.len(), collected);
    }
    collected
}
