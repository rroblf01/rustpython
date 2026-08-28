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
    pub(crate) fn install_source_defined_stdlib(&mut self, module_name: &str, source: &str, names: &[&str]) {
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
        if self
            .exec_code(code, Some(Rc::clone(&dedicated_globals)))
            .is_err()
        {
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
}
