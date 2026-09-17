use crate::bytecode::CodeObject;
use crate::compiler::Compiler;
use crate::interner;
use crate::object::*;
use crate::parser::Parser;
use crate::vm::VirtualMachine;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

impl VirtualMachine {
    /// Some stdlib classes are far easier (and more correct) to express as
    /// real Python source — the same way CPython's own stdlib does it — than
    /// as hand-written Rust closures (e.g. anything relying on composition
    /// over a `self.data` attribute, decorators, or `with`). This compiles
    /// and runs that source against its own dedicated, isolated globals
    /// dict (never `self.globals` — the VM's real, shared top-level
    /// namespace) and merges the requested names into the given
    /// already-registered native module's dict. Using a dedicated dict
    /// (rather than running against `self.globals` and stripping the
    /// requested names back out afterward, which this used to do) matters
    /// for correctness, not just tidiness: any exported function/class
    /// whose body references ANOTHER exported name (e.g. gettext's
    /// `translation()` referencing `GNUTranslations`) keeps working
    /// correctly forever, since the dict its closure captured is never
    /// mutated again — stripping names out from underneath it broke exactly
    /// this pattern.
    pub(crate) fn install_source_defined_stdlib(
        &mut self,
        module_name: &str,
        source: &str,
        names: &[&str],
    ) {
        // Every `VirtualMachine::new()` — including the many *disposable*
        // ones spun up for nested Python-level calls from Rust builtin code
        // (a separate, documented architectural gap — see
        // `call_bound_method`'s doc comments) — used to re-parse, re-compile,
        // AND re-EXECUTE this same, never-changing Python source from
        // scratch. That's cheap in isolation but catastrophic under real
        // workloads: a single Django import chain observed here triggers
        // 2000+ disposable VMs, each redoing all of this, dominating a
        // 56-SECOND import of `django.db.models`. First fixed the
        // parse+compile half by caching the compiled `CodeObject` (cut it to
        // ~28s) — this caches the EXECUTION RESULT too: the extracted
        // `(name, PyObjectRef)` pairs (e.g. `collections.Counter`,
        // `enum.Enum` — real Type objects with their own dict of methods)
        // are stored once and simply Rc-cloned into every subsequent VM's
        // module dict instead of re-running ~800 lines of bytecode (class
        // bodies, method definitions, docstrings) per VM. This is safe
        // because these are conceptually process-global stdlib singletons —
        // exactly how CPython's own module system treats them (one `Counter`
        // class shared by every importer) — sharing the actual objects
        // across VM instances doesn't change any observable behavior for
        // user code (isinstance/identity/method dispatch all still work,
        // since it's genuinely the same objects everywhere).
        thread_local! {
            static COMPILED_STDLIB_CACHE: std::cell::RefCell<HashMap<String, Rc<CodeObject>>> = std::cell::RefCell::new(HashMap::new());
            static EXECUTED_STDLIB_CACHE: std::cell::RefCell<HashMap<String, Rc<Vec<(String, PyObjectRef)>>>> = std::cell::RefCell::new(HashMap::new());
        }
        if let Some(cached_extracted) =
            EXECUTED_STDLIB_CACHE.with(|c| c.borrow().get(module_name).cloned())
        {
            if let Some(module) = self.modules.get(module_name) {
                if let PyObject::Module { dict, .. } = &mut *module.borrow_mut() {
                    for (name, obj) in cached_extracted.iter() {
                        dict.insert_str(name, obj.clone());
                    }
                }
            }
            return;
        }
        let cached = COMPILED_STDLIB_CACHE.with(|c| c.borrow().get(module_name).cloned());
        let code = match cached {
            Some(rc) => (*rc).clone(),
            None => {
                let mut parser = Parser::new(source);
                let program = match parser.parse_program() {
                    Ok(p) => p,
                    Err(_) => return,
                };
                let mut compiler = Compiler::new();
                let code = match compiler.compile(&program, &format!("<{}>", module_name)) {
                    Ok(c) => c,
                    Err(_) => return,
                };
                COMPILED_STDLIB_CACHE.with(|c| {
                    c.borrow_mut()
                        .insert(module_name.to_string(), Rc::new(code.clone()))
                });
                code
            }
        };
        // Real modules always have __name__ in their globals — class bodies
        // compiled inside this source (e.g. collections.Counter, a real
        // `class Counter(dict): ...`) now implicitly do `__module__ =
        // __name__` as their first statement (see compile_class_body), which
        // would otherwise NameError here since this dict starts empty.
        let dedicated_globals = Rc::new(RefCell::new(HashMap::from([(
            interner::intern("__name__"),
            py_str(module_name),
        )])));
        if let Err(e) = self.exec_code(code, Some(Rc::clone(&dedicated_globals))) {
            // This used to be a silent `return` — a real exception raised
            // ANYWHERE in `source`'s top-level execution (a typo, a
            // genuine bug in newly-added code, ...) meant `module_name`'s
            // native module ended up with NONE of `names` populated (not
            // even ones successfully defined by statements BEFORE the
            // failure — `extracted`, built from the whitelist below, is
            // only ever reached on the success path), and neither the
            // caller (`VirtualMachine::new()`, itself infallible/`-> Self`
            // by construction) nor the user ever saw so much as a hint
            // that anything had gone wrong: `import enum` (or `functools`/
            // `http`/`gettext`/`contextvars`, the other consumers of this
            // same mechanism) would just silently come back missing
            // whatever names the source was SUPPOSED to define, with
            // `import` itself reporting success. Real trigger: a bug
            // introduced into `enum_extra.py` while adding `Flag`/`IntFlag`
            // support broke the WHOLE `enum` module this way — `Enum`,
            // defined and working near the TOP of that file, silently
            // vanished from `enum.Enum` because of a `TypeError` raised by
            // unrelated code hundreds of lines later. Making this loud
            // (rather than fully propagating — this function has no
            // `PyResult` return type, and giving it one means threading a
            // `Result` all the way through the many `VirtualMachine::new()`
            // call sites that invoke it unconditionally, a much larger
            // change than this bug fix warrants) at least turns a
            // completely invisible failure into an immediately-visible
            // one — printed once per module name per process (not once
            // per attempt) since a failure this early tends to recur on
            // every one of the many *disposable* VMs this same file's
            // module doc comment describes (a single `-c` invocation can
            // construct dozens of them), and printing unconditionally on
            // every one of those turned one real failure into a wall of
            // identical repeated warnings.
            thread_local! {
                static WARNED_MODULES: std::cell::RefCell<std::collections::HashSet<String>> =
                    std::cell::RefCell::new(std::collections::HashSet::new());
            }
            let first_time =
                WARNED_MODULES.with(|w| w.borrow_mut().insert(module_name.to_string()));
            // Gated behind an opt-in env var, NOT printed unconditionally:
            // this failure mode is expected and harmless on every single
            // process's very first `VirtualMachine::new()` (see
            // `install_collections_abc_alias`'s own reentrancy-guard
            // comment — a nested disposable VM built WHILE the outer
            // `collections.abc` bootstrap is still in flight always hits
            // this exact `ModuleNotFoundError` for any stdlib-extra source
            // that itself does `import collections.abc`, e.g.
            // `contextvars`'s bootstrap script; the OUTER/real VM that
            // actually runs user code still gets a fully-populated
            // `contextvars.Context` once its own, non-reentrant
            // `collections.abc` import completes). Printing this
            // unconditionally turned into stderr noise on the FIRST
            // `VirtualMachine::new()` of every process regardless of what
            // the user's script does (confirmed: even `print(1)` triggered
            // it) — which also corrupts `tests/cpython_runner.sh`'s
            // first-non-JIT-line PARSE_ERROR heuristic. Set
            // `RPY_DEBUG_STDLIB_BOOTSTRAP=1` to see it when actually
            // debugging a NEW failure in this mechanism.
            if first_time && std::env::var("RPY_DEBUG_STDLIB_BOOTSTRAP").is_ok() {
                eprintln!(
                    "warning: install_source_defined_stdlib({module_name:?}) failed: {e}\n  \
                     the '{module_name}' module will be missing some or all of {names:?}",
                );
            }
            return;
        }
        let extracted: Vec<(String, PyObjectRef)> = {
            let globals = dedicated_globals.borrow();
            names
                .iter()
                .filter_map(|name| {
                    globals
                        .get(&interner::intern(name))
                        .cloned()
                        .map(|v| (name.to_string(), v))
                })
                .collect()
        };
        EXECUTED_STDLIB_CACHE.with(|c| {
            c.borrow_mut()
                .insert(module_name.to_string(), Rc::new(extracted.clone()))
        });
        if let Some(module) = self.modules.get(module_name) {
            if let PyObject::Module { dict, .. } = &mut *module.borrow_mut() {
                for (name, obj) in extracted {
                    dict.insert_str(&name, obj);
                }
            }
        }
    }

    /// Wire up `collections.abc` against the real, vendored
    /// `Lib/_collections_abc.py` (built through real `abc.ABCMeta`),
    /// mirroring what real CPython's `collections/__init__.py` does at
    /// import time:
    /// ```python
    /// import _collections_abc
    /// sys.modules['collections.abc'] = _collections_abc
    /// abc = _collections_abc
    /// ```
    /// The native `collections` module here has no Python source of its own
    /// to run that in, and — unlike a plain dotted submodule — `collections`
    /// has no `__path__` for the normal dotted-import walker to find a
    /// `collections/abc.py` under (real CPython doesn't have one either as
    /// of 3.14: this alias is the only way `collections.abc` resolves).
    ///
    /// Only actually IMPORTS `_collections_abc` (parsing + compiling +
    /// running ~1200 lines of real stdlib source, including `abc.ABCMeta`
    /// class-creation machinery) once per process/thread: the resulting
    /// module object is cached and Rc-shared into every subsequent
    /// `VirtualMachine::new()` call, the same "process-wide stdlib
    /// singleton" pattern `install_source_defined_stdlib`'s own
    /// `EXECUTED_STDLIB_CACHE` and `VM_STATE_CACHE` already use — without
    /// this, every one of the (potentially thousands of) disposable VMs
    /// spun up for nested Python-level calls would re-run the whole module
    /// from scratch.
    pub(crate) fn install_collections_abc_alias(&mut self) {
        thread_local! {
            static CACHED_ABC_MODULE: RefCell<Option<PyObjectRef>> = RefCell::new(None);
            // Re-entrancy guard — see the long comment below for why this
            // is load-bearing, not defensive: without it, this hangs on
            // EVERY invocation of the interpreter.
            static IMPORTING: std::cell::Cell<bool> = std::cell::Cell::new(false);
        }
        if self.modules.contains_key("collections.abc") {
            return;
        }
        let abc_mod = if let Some(cached) = CACHED_ABC_MODULE.with(|c| c.borrow().clone()) {
            cached
        } else {
            // Importing `_collections_abc` for the very first time compiles
            // and executes ~1200 lines of real stdlib source — class bodies
            // (`class Mapping(Collection, metaclass=ABCMeta): ...`),
            // `abc.ABCMeta.__new__` machinery, `async def`/generator
            // bootstrapping, etc. Some of that machinery, along the way,
            // constructs its OWN nested disposable `VirtualMachine`s (the
            // same "many *disposable* VMs spun up for nested Python-level
            // calls from Rust builtin code" pattern documented on
            // `install_source_defined_stdlib`) — and every
            // `VirtualMachine::new()`, including those nested ones, reaches
            // this exact method (from `vm.rs`'s "fast path" branch, since
            // by then `VM_STATE_CACHE` is already populated). Without this
            // guard, each such nested call — landing here BEFORE the
            // outer, still-in-progress import has had a chance to populate
            // `CACHED_ABC_MODULE` — saw an empty cache and kicked off its
            // OWN full re-import, which spun up more nested VMs, which did
            // the same, unboundedly: confirmed as a hang on literally
            // every `-c` invocation (even `print('hello')`, since
            // `VirtualMachine::new()` runs unconditionally) once this
            // method's fast-path call site was enabled. Skipping the
            // import on a re-entrant call is safe: that inner disposable
            // VM simply proceeds without `collections.abc` pre-wired
            // (falling back to whatever a real `import collections.abc`
            // statement inside its own code would do, same as any other
            // not-yet-cached stdlib extra), and the ONE outer, non-
            // re-entrant call completes normally and populates the cache
            // for every VM constructed after it returns.
            if IMPORTING.with(|f| f.get()) {
                return;
            }
            IMPORTING.with(|f| f.set(true));
            let result = self.import_module_from_file("_collections_abc");
            IMPORTING.with(|f| f.set(false));
            match result {
                Ok(m) => {
                    CACHED_ABC_MODULE.with(|c| *c.borrow_mut() = Some(m.clone()));
                    m
                }
                Err(_) => return,
            }
        };
        self.modules
            .insert("_collections_abc".to_string(), abc_mod.clone());
        self.modules
            .insert("collections.abc".to_string(), abc_mod.clone());
        if let Some(collections_mod) = self.modules.get("collections").cloned() {
            if let PyObject::Module { dict, .. } = &mut *collections_mod.borrow_mut() {
                dict.insert_str("abc", abc_mod.clone());
            }
        }
        if let Some(sys_mod) = self.modules.get("sys") {
            let mod_dict = {
                let b = sys_mod.borrow();
                if let PyObject::Module { dict, .. } = &*b {
                    dict.get_str("modules").cloned()
                } else {
                    None
                }
            };
            if let Some(mod_dict) = mod_dict {
                if let PyObject::Dict(ref mut d) = &mut *mod_dict.borrow_mut() {
                    let _ = d.set(py_str("_collections_abc"), abc_mod.clone());
                    let _ = d.set(py_str("collections.abc"), abc_mod.clone());
                }
            }
        }
    }

    /// Marks `module_name.class_name`'s own type dict with
    /// `NO_SUBCLASS_KEY`, rejecting `class X(that_class): ...` the same way
    /// `bool` is rejected — used for classes built via a small
    /// `install_source_defined_stdlib` snippet (e.g. `contextvars.Context`,
    /// which is real CPython's own — and this codebase's — disallowed
    /// subclassing case) where the marker can't be set directly on the
    /// native type at construction time, since the snippet's OWN class
    /// statement needs to subclass a related native base first.
    pub(crate) fn stamp_no_subclass(
        &mut self,
        module_name: &str,
        class_name: &str,
    ) -> Option<PyObjectRef> {
        let module = self.modules.get(module_name)?;
        let cls = if let PyObject::Module { dict, .. } = &*module.borrow() {
            dict.get_str(class_name).cloned()
        } else {
            None
        }?;
        if let PyObject::Type { dict, .. } = &mut *cls.borrow_mut() {
            dict.insert_str(crate::object::NO_SUBCLASS_KEY, py_bool(true));
        }
        Some(cls)
    }
}
