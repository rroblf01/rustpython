use crate::bytecode::*;
use crate::interner::{self, InternedMap, StrId};
use num_bigint::BigInt;
use num_traits::ToPrimitive;
use smallvec::SmallVec;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::compiler::Compiler;
#[cfg(feature = "jit")]
use crate::jit::JitCompiler;
use crate::modules::*;
use crate::object::*;
use crate::parser::Parser;

pub mod frame;
pub use frame::{ExceptionHandler, Frame};
pub mod helpers;
pub(crate) use helpers::{
    deref_proxy, eval_const_value, find_lib_dir, formal_param_index, inplace_binary_op,
    plain_binary_op, unbound_local_msg,
};
pub mod machine;
pub use machine::VirtualMachine;
pub mod pool;
pub mod import;
pub mod except;
pub mod class;
pub mod format;
pub use format::format_with_spec;
pub mod descriptor;
pub mod finalizer;
pub mod stdlib;
pub mod util;
pub(crate) use util::{exc_type_matches, is_exception_subclass, OPCODE_HIST, OPCODE_HIST_ENABLED};
pub(crate) use util::{get_shared_builtins_module, opcode_hist_dump, opcode_hist_init_from_env, set_sys_modules_priority};
pub mod run;
pub mod execute;
pub mod call;
pub mod disposable;
pub mod ops;
pub mod iter;
pub mod op_import;
pub mod op_coll;
pub mod op_unpack;
pub mod op_reg;
pub mod op_with;
pub mod op_stack;
pub mod op_call;
pub mod op_var;
pub mod op_exc;
pub mod op_attr;
pub mod op_attr_helpers;
pub mod op_store;
pub mod dispatch;
pub mod call_class;
pub mod init;
pub mod call_func;

thread_local! {
    pub(crate) static ATTR_CACHE: std::cell::RefCell<HashMap<(String, String), crate::object::BuiltinFunc>> = std::cell::RefCell::new(HashMap::new());
}

thread_local! {
    pub(crate) static SHARED_BUILTINS_MODULE_REF: RefCell<Option<PyObjectRef>> = const { RefCell::new(None) };
}

thread_local! {
    pub(crate) static SYS_MODULES_PRIORITY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

impl VirtualMachine {
    pub fn new() -> Self {
        if std::env::var("RPY_DEBUG_VM_NEW").is_ok() {
            eprintln!("VM_NEW");
        }
        Self::new_with_args(std::env::args().collect())
    }

    pub fn new_with_args(argv: Vec<String>) -> Self {
        // Constructing a full VM means running EVERY native module
        // constructor below (~100+ modules) plus filesystem-based venv/
        // site-packages detection — cheap for the ONE real, top-level
        // program VM, but catastrophic for the many *disposable* VMs
        // spun up just to resume one step of a generator/coroutine
        // (`generator_next_fallback`, `coroutine_send_fallback` below —
        // see their own doc comments on why a disposable VM is used at
        // all) since `next()` on a generator does this on EVERY SINGLE
        // resume. Confirmed via a minimal repro: a no-op
        // `@contextlib.contextmanager` (whose `__enter__`/`__exit__` each
        // call `next()` once) entered/exited 14,202 times in a bare loop
        // took 30+ seconds — should be near-instant, and this was the
        // actual root cause behind several CPython test files (test_math,
        // test_statistics, ...) appearing to hang. Real CPython has
        // exactly ONE `sys`/`os`/etc. module per process for its entire
        // lifetime, shared by every generator resume — rebuilding them
        // fresh per resume was never semantically necessary, just an
        // artifact of how disposable-VM construction happened to be
        // written. Caching the built `(builtins, modules)` state after the
        // FIRST construction and Rc-cloning it into every subsequent
        // `new()` call matches that real-CPython "one shared stdlib per
        // process" model directly, and follows the same pattern already
        // established (and proven safe) by `install_source_defined_stdlib`'s
        // own `EXECUTED_STDLIB_CACHE` further below — sharing the actual
        // module/function objects across VM instances doesn't change any
        // observable behavior for ordinary user code (isinstance/identity/
        // method dispatch all keep working, since it's genuinely the same
        // objects everywhere, exactly as real CPython's own process-wide
        // module singletons behave). Safe regardless of `argv`: the ONE
        // real top-level VM (constructed directly with the process's real
        // `sys.argv` in `main.rs`) always runs first and populates this
        // cache; every later disposable VM goes through `new()`, which
        // passes that same real `std::env::args()` anyway.
        thread_local! {
            static VM_STATE_CACHE: RefCell<Option<(Rc<HashMap<StrId, PyObjectRef>>, HashMap<String, PyObjectRef>)>> = RefCell::new(None);
        }
        if let Some((cached_builtins, cached_modules)) = VM_STATE_CACHE.with(|c| c.borrow().clone())
        {
            let globals_map: HashMap<StrId, PyObjectRef> = HashMap::from([
                (interner::intern("__name__"), py_str("__main__")),
                (
                    interner::intern("__builtins__"),
                    create_module(
                        "builtins",
                        cached_builtins
                            .iter()
                            .map(|(k, v)| (interner::lookup_str(*k).to_string(), v.clone()))
                            .collect::<HashMap<String, PyObjectRef>>(),
                    ),
                ),
            ]);
            let globals = Rc::new(RefCell::new(globals_map));
            let mut vm = VirtualMachine {
                frames: Vec::new(),
                builtins: Rc::clone(&cached_builtins),
                modules: cached_modules,
                globals,
                #[cfg(feature = "jit")]
                jit: RefCell::new(JitCompiler::new()),
                profile: RefCell::new(HashMap::new()),
                last_error_line: None,
                last_error_file: None,
                last_traceback: Vec::new(),
                frame_pool: Vec::new(),
                type_registry: HashMap::new(),
                exc_type: None,
                exc_value: None,
                exc_traceback: None,
                exc_context_stack: Vec::new(),
                propagating_exc: None,
                recursion_limit: 1000,
            };
            vm.populate_type_registry();
            vm.install_source_defined_stdlib(
                "collections",
                crate::modules::COLLECTIONS_USER_TYPES_SOURCE,
                &[
                    "UserList",
                    "UserDict",
                    "UserString",
                    "Counter",
                    "defaultdict",
                    "ChainMap",
                ],
            );
            vm.install_source_defined_stdlib(
                "functools",
                crate::modules::FUNCTOOLS_EXTRA_SOURCE,
                &["lru_cache", "cache"],
            );
            vm.install_source_defined_stdlib(
                "enum",
                crate::modules::ENUM_SOURCE,
                &[
                    "auto",
                    "nonmember",
                    "member",
                    "property",
                    "EnumType",
                    "EnumMeta",
                    "Enum",
                    "IntEnum",
                    "StrEnum",
                    "unique",
                    "_simple_enum",
                    "_test_simple_enum",
                ],
            );
            vm.install_source_defined_stdlib(
                "http",
                crate::modules::HTTP_SOURCE,
                &["HTTPStatus", "HTTPMethod"],
            );
            vm.install_source_defined_stdlib(
                "gettext",
                crate::modules::GETTEXT_SOURCE,
                &[
                    "NullTranslations",
                    "GNUTranslations",
                    "find",
                    "translation",
                    "install",
                    "textdomain",
                    "bindtextdomain",
                    "gettext",
                    "ngettext",
                    "pgettext",
                    "npgettext",
                    "dgettext",
                    "dngettext",
                    "_localedirs",
                    "_current_domain",
                    "_default_localedir",
                    "__all__",
                ],
            );
            vm.install_source_defined_stdlib(
                "json",
                crate::modules::JSON_EXTRA_SOURCE,
                &["JSONEncoder", "dumps"],
            );
            return vm;
        }

        let builtins_str_map = create_builtins();
        let mut builtins: HashMap<StrId, PyObjectRef> = str_map_to_strid_map(builtins_str_map);
        let builtins_to_module = |map: &HashMap<StrId, PyObjectRef>| {
            map.iter()
                .map(|(k, v)| (interner::lookup_str(*k).to_string(), v.clone()))
                .collect::<HashMap<String, PyObjectRef>>()
        };
        // ONE shared `builtins` module for sys.modules, globals' `__builtins__`,
        // and `import builtins` — mutations (`builtins.len = f`, test_dynamic)
        // must be visible through every path, so they must all wrap the same
        // dict rather than separate copies.
        let builtins_module = create_module("builtins", builtins_to_module(&builtins));
        let globals_map: HashMap<StrId, PyObjectRef> = HashMap::from([
            (interner::intern("__name__"), py_str("__main__")),
            (interner::intern("__builtins__"), builtins_module.clone()),
        ]);
        let globals = Rc::new(RefCell::new(globals_map));

        let mut modules: HashMap<String, PyObjectRef> = HashMap::new();
        SHARED_BUILTINS_MODULE_REF.with(|c| *c.borrow_mut() = Some(builtins_module.clone()));
        modules.insert_str("builtins", builtins_module);
        modules.insert_str("math", create_module("math", create_math_dict()));
        modules.insert_str("_codecs", create_module("_codecs", create_codecs_dict()));

        let mut sys_dict = create_sys_dict(argv);
        // sys.path is shared (Rc-cloned) across every VirtualMachine
        // instance in this process/thread, real or disposable — see the
        // populate-defaults block below for why. `sys.modules` is
        // deliberately NOT shared this way: it's populated per-VM below
        // from that VM's own already-built `modules` map, which is the
        // correct, VM-local behavior for module caching.
        thread_local! {
            static SHARED_SYS_PATH: RefCell<Option<PyObjectRef>> = RefCell::new(None);
        }
        let reused_shared_path = SHARED_SYS_PATH.with(|c| c.borrow().clone());
        if let Some(shared_path) = reused_shared_path.clone() {
            sys_dict.insert_str("path", shared_path);
        }
        // sys.meta_path — import hooks
        if !sys_dict.contains_key("meta_path") {
            let meta_path = py_list(vec![PyObjectRef::imm(PyObject::BuiltinFunction {
                name: "BuiltinImporter".to_string(),
                func: |args| {
                    if args.len() < 2 {
                        return Err(PyError::type_error("find_spec() requires 2 arguments"));
                    }
                    Err(PyError::module_not_found_error(format!(
                        "No module named '{}'",
                        args[1].str()
                    )))
                },
            })]);
            sys_dict.insert_str("meta_path", meta_path);
        }
        if !sys_dict.contains_key("path_hooks") {
            sys_dict.insert_str("path_hooks", py_list(vec![]));
        }
        if !sys_dict.contains_key("path_importer_cache") {
            sys_dict.insert_str("path_importer_cache", py_dict());
        }
        modules.insert_str("sys", create_module("sys", sys_dict.clone()));
        for (k, v) in sys_dict.clone() {
            builtins.insert(interner::intern(&k), v);
        }

        // Share the sys module with native code that must read the CURRENT
        // `sys.unraisablehook` (atexit's `_run_exitfuncs` reports a raising
        // callback through it — the hook may have been reassigned by
        // `catch_warnings`-style contexts like test.support's
        // `catch_unraisable_exception`, so it must be read live, not
        // captured at module-creation time).
        crate::modules::set_sys_module(modules.get("sys").cloned());

        crate::vm::init::register_native_modules(&mut modules, &builtins);

        // Native sqlite3 module (requires --features sqlite3)
        #[cfg(feature = "sqlite3")]
        {
            let sqlite3_mod = create_module("sqlite3", create_sqlite3_dict());
            modules.insert_str("sqlite3", sqlite3_mod.clone());
            // sqlite3.dbapi2 — real CPython's sqlite3 package re-exports
            // everything under this name too (the legacy PEP 249 DB-API
            // 2.0 module alias). Real code: Django's own
            // `django/db/backends/sqlite3/base.py` does `from sqlite3
            // import dbapi2 as Database`. Same module object, not a
            // separate copy — matches how CPython's own `dbapi2.py` is
            // just `from sqlite3.dbapi2 import *`-equivalent re-exports.
            modules.insert_str("sqlite3.dbapi2", sqlite3_mod);
        }

        // Populate sys.path with default search paths — ONLY the first
        // time this process/thread creates a VM. Every subsequent
        // VirtualMachine::new() (crucially including the many *disposable*
        // ones `call_bound_method` spins up per nested Python-level call —
        // a separate, documented architectural gap) reuses the exact same
        // list object via `reused_shared_path` above instead of rebuilding
        // it from scratch. This fixes a real correctness bug (a disposable
        // VM's sys.path previously reverted to just these hardcoded
        // defaults, invisible to whatever `sys.path.insert(...)` the
        // running script had done against the real VM — breaking any
        // nested call that needed to import a real site-packages module,
        // confirmed via Django's `django.db.utils.load_backend`) and a
        // real performance win (this block does venv detection + `.pth`
        // file filesystem I/O, previously repeated on every one of 2000+
        // disposable VMs in one observed Django import).
        if reused_shared_path.is_none() {
            if let PyObject::List(path_list) = &mut *sys_dict.get("path").unwrap().borrow_mut() {
                path_list.push(py_str("."));
                path_list.push(py_str(&find_lib_dir()));

                // Read PYTHONPATH environment variable
                if let Ok(pythonpath) = std::env::var("PYTHONPATH") {
                    for p in pythonpath.split(':') {
                        let trimmed = p.trim();
                        if !trimmed.is_empty() {
                            path_list.push(py_str(trimmed));
                        }
                    }
                }

                // Detect virtual environment (VIRTUAL_ENV, conda, poetry, pixi, or .venv in CWD)
                let venv = std::env::var("VIRTUAL_ENV")
                    .ok()
                    .or_else(|| std::env::var("CONDA_PREFIX").ok())
                    .or_else(|| {
                        if std::env::var("POETRY_ACTIVE").is_ok() {
                            std::env::var("POETRY_VIRTUAL_ENV").ok()
                        } else {
                            None
                        }
                    })
                    .or_else(|| {
                        std::env::var("PIXI_IN_SHELL")
                            .ok()
                            .and_then(|_| std::env::var("PIXI_PROJECT_ROOT").ok())
                    })
                    .or_else(|| {
                        let cwd = std::env::current_dir().ok();
                        if cfg!(feature = "profile") {
                            eprintln!("DEBUG venv: VIRTUAL_ENV not set, checking CWD .venv");
                        }
                        if let Some(ref d) = cwd {
                            let dotvenv = d.join(".venv");
                            if cfg!(feature = "profile") {
                                eprintln!(
                                    "DEBUG venv: checking {}. is_dir={}",
                                    dotvenv.display(),
                                    dotvenv.is_dir()
                                );
                            }
                        }
                        cwd.filter(|d| d.join(".venv").is_dir())
                            .map(|d| d.join(".venv").to_string_lossy().to_string())
                    });

                if let Some(ref venv_path) = venv {
                    // Try to read pyvenv.cfg to determine the Python version
                    let py_version = std::fs::read_to_string(format!("{}/pyvenv.cfg", venv_path))
                        .ok()
                        .and_then(|cfg| {
                            for line in cfg.lines() {
                                if let Some(ver) = line.strip_prefix("version = ") {
                                    // Parse "3.13.2" -> "3.13"
                                    let parts: Vec<&str> = ver.splitn(2, '.').collect();
                                    if parts.len() == 2 {
                                        let major_minor = if let Some(dot2) = parts[1].find('.') {
                                            &parts[1][..dot2]
                                        } else {
                                            parts[1]
                                        };
                                        return Some(format!("{}.{}", parts[0], major_minor));
                                    }
                                }
                            }
                            None
                        })
                        .unwrap_or_else(|| "3.13".to_string());

                    // Add site-packages directory
                    let site_pkg = format!("{}/lib/python{}/site-packages", venv_path, py_version);
                    if std::path::Path::new(&site_pkg).is_dir() {
                        path_list.push(py_str(&site_pkg));

                        // Process .pth files in site-packages (e.g., easy-install.pth, distutils-precedence.pth)
                        if let Ok(entries) = std::fs::read_dir(&site_pkg) {
                            for entry in entries.flatten() {
                                let entry_path = entry.path();
                                if entry_path.extension().map_or(false, |e| e == "pth") {
                                    if let Ok(content) = std::fs::read_to_string(&entry_path) {
                                        for line in content.lines() {
                                            let trimmed = line.trim();
                                            if trimmed.is_empty() || trimmed.starts_with('#') {
                                                continue;
                                            }
                                            if trimmed.starts_with('.') || trimmed.starts_with('/')
                                            {
                                                let resolved = if trimmed.starts_with('.') {
                                                    format!("{}/{}", site_pkg, trimmed)
                                                } else {
                                                    trimmed.to_string()
                                                };
                                                if !path_list
                                                    .iter()
                                                    .any(|p| p.borrow().str() == resolved)
                                                {
                                                    path_list.push(py_str(&resolved));
                                                }
                                            }
                                            // 'import' directives in .pth are skipped for now
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        // First VM in this process/thread: publish the freshly-populated
        // path list so every subsequent VirtualMachine::new() reuses it.
        if reused_shared_path.is_none() {
            let path_list = sys_dict.get("path").unwrap().clone();
            SHARED_SYS_PATH.with(|c| *c.borrow_mut() = Some(path_list));
        }
        // Populate sys.modules with already-loaded modules
        if let PyObject::Dict(mod_dict) = &mut *sys_dict.get("modules").unwrap().borrow_mut() {
            for (name, module) in &modules {
                mod_dict.set(py_str(name), module.clone()).ok();
            }
        }

        // Wrap builtins in Rc for sharing across frames
        let builtins = Rc::new(builtins);

        // Share the real builtins map so native code can resolve a builtin
        // exception CLASS by name (type() of a PyObject::Exception must
        // return the real ZeroDivisionError & co., not a synthetic Type —
        // test_atexit's `type(exc_value) == ZeroDivisionError`).
        crate::modules::set_builtins_ref(Rc::clone(&builtins));
        crate::modules::register_collections_abc_builtins();

        // Populate the disposable-VM fast path's cache (see the doc
        // comment at the top of this function) — safe to do BEFORE the
        // `install_source_defined_stdlib` calls below even though those
        // still mutate module dicts afterward: this clones the `modules`
        // HashMap's *entries* (Rc-bumps), not the module objects
        // themselves, so any mutation those calls make lands on the same
        // shared objects this cache already points to.
        VM_STATE_CACHE.with(|c| *c.borrow_mut() = Some((Rc::clone(&builtins), modules.clone())));

        let mut vm = VirtualMachine {
            frames: Vec::new(),
            builtins: Rc::clone(&builtins),
            modules,
            globals,
            #[cfg(feature = "jit")]
            jit: RefCell::new(JitCompiler::new()),
            profile: RefCell::new(HashMap::new()),
            last_error_line: None,
            last_error_file: None,
            last_traceback: Vec::new(),
            frame_pool: Vec::new(),
            type_registry: HashMap::new(),
            exc_type: None,
            exc_value: None,
            exc_traceback: None,
            exc_context_stack: Vec::new(),
            propagating_exc: None,
            recursion_limit: 1000,
        };
        vm.populate_type_registry();
        vm.install_source_defined_stdlib(
            "collections",
            crate::modules::COLLECTIONS_USER_TYPES_SOURCE,
            &[
                "UserList",
                "UserDict",
                "UserString",
                "Counter",
                "defaultdict",
                "ChainMap",
                "_count_elements",
            ],
        );
        // contextlib no longer native — real Lib/contextlib.py already defines ContextDecorator
        vm.install_source_defined_stdlib(
            "functools",
            crate::modules::FUNCTOOLS_EXTRA_SOURCE,
            &["lru_cache", "cache"],
        );
        vm.install_source_defined_stdlib(
            "enum",
            crate::modules::ENUM_SOURCE,
            &[
                "auto",
                "nonmember",
                "member",
                "property",
                "EnumType",
                "EnumMeta",
                "Enum",
                "IntEnum",
                "StrEnum",
                "unique",
                "_simple_enum",
                "_test_simple_enum",
            ],
        );
        vm.install_source_defined_stdlib(
            "gettext",
            crate::modules::GETTEXT_SOURCE,
            &[
                "NullTranslations",
                "GNUTranslations",
                "find",
                "translation",
                "install",
                "textdomain",
                "bindtextdomain",
                "gettext",
                "ngettext",
                "pgettext",
                "npgettext",
                "dgettext",
                "dngettext",
                "_localedirs",
                "_current_domain",
                "_default_localedir",
                "__all__",
            ],
        );
        vm.install_source_defined_stdlib(
            "json",
            crate::modules::JSON_EXTRA_SOURCE,
            &["JSONEncoder", "dumps"],
        );
        vm
    }



    pub(crate) fn call_function(
        &mut self,
        callable: PyObjectRef,
        args: Vec<PyObjectRef>,
        keywords: Vec<(String, PyObjectRef)>,
    ) -> PyResult<PyObjectRef> {
        let type_name = callable.borrow().type_name();
        if cfg!(feature = "profile") {
            eprintln!(
                "DEBUG call_function: type={} name={:?}",
                type_name,
                callable.repr()
            );
        }
        if std::env::var("RPY_DEBUG_CALL").is_ok() {
            eprintln!("CALL_FUNCTION: type={} repr={}", type_name, callable.repr());
        }
        if let Some(val) = self.try_handle_special_builtin(&callable, &args, &keywords)? {
            return Ok(val);
        }

        if let PyObject::BuiltinFunction { func, .. } = &*callable.borrow() {
            let func = *func;
            // `Fraction(...)` construction needs a live `&mut self` so it can
            // invoke user-provided `as_integer_ratio()` methods (same pattern
            // as getmembers/find_spec above: avoids reborrowing a disposable
            // VM over the live one from inside an active call chain).
            let is_fraction_init = std::ptr::fn_addr_eq(
                func,
                crate::modules::fraction_init_fallback as crate::object::BuiltinFunc,
            );
            if is_fraction_init {
                return crate::modules::fraction_init_with_vm(self, &args);
            }
            if std::ptr::fn_addr_eq(
                func,
                crate::modules::fraction_from_number_fallback as crate::object::BuiltinFunc,
            ) {
                return crate::modules::fraction_from_number_with_vm(self, &args);
            }
            if std::ptr::fn_addr_eq(
                func,
                crate::modules::fraction_from_decimal_fallback as crate::object::BuiltinFunc,
            ) {
                return crate::modules::fraction_from_decimal_with_vm(self, &args);
            }
            // Pack keyword arguments into a dict and append as last arg
            if !keywords.is_empty() {
                let mut dict = crate::object::PyDict::new();
                for (k, v) in &keywords {
                    let _ = dict.set(crate::object::py_str(k), v.clone());
                }
                // `list(sequence=...)`/`tuple(sequence=...)` must TypeError,
                // but a POSITIONAL dict is fine (`list({'a':1})` ->
                // `['a']`) — the flattened trailing-dict convention can't
                // distinguish them inside the builtins, so the keyword
                // rejection lives HERE, where `keywords` is known (only fires
                // when keywords exist).
                if std::ptr::fn_addr_eq(
                    func,
                    crate::object::builtin_list as crate::object::BuiltinFunc,
                ) || std::ptr::fn_addr_eq(
                    func,
                    crate::object::builtin_tuple as crate::object::BuiltinFunc,
                ) {
                    return Err(PyError::type_error(format!(
                        "{}() takes no keyword arguments",
                        if std::ptr::fn_addr_eq(
                            func,
                            crate::object::builtin_list as crate::object::BuiltinFunc,
                        ) {
                            "list"
                        } else {
                            "tuple"
                        }
                    )));
                }
                let mut new_args = args;
                new_args.push(crate::object::PyObjectRef::new(
                    crate::object::PyObject::Dict(Box::new(dict)),
                ));
                return func(&new_args);
            }
            return func(&args);
        }

        if let PyObject::BuiltinMethod { func, self_obj, .. } = &*callable.borrow() {
            let func = *func;
            let self_obj = self_obj.clone();
            let mut new_args = vec![self_obj];
            new_args.extend(args);
            if !keywords.is_empty() {
                let mut dict = crate::object::PyDict::new();
                for (k, v) in keywords {
                    let _ = dict.set(crate::object::py_str(&k), v);
                }
                new_args.push(crate::object::PyObjectRef::new(
                    crate::object::PyObject::Dict(Box::new(dict)),
                ));
            }
            // `generator.throw()` needs real `&mut self` access so the
            // resumed generator body's `sys.exc_info()` sees THIS VM's
            // exc_type/exc_value (set moments earlier by the original
            // `raise`) instead of a disposable VM's blank ones — see
            // `generator_throw_with_vm`'s own doc comment.
            if std::ptr::fn_addr_eq(
                func,
                crate::object::generator_throw_fallback as crate::object::BuiltinFunc,
            ) {
                return crate::object::generator_throw_with_vm(self, &new_args);
            }
            // `Fraction.__init__` bound to an instance (reached via the
            // type-call machinery's BoundMethod path) — same VM-routing as
            // the BuiltinFunction arm above.
            if std::ptr::fn_addr_eq(
                func,
                crate::modules::fraction_init_fallback as crate::object::BuiltinFunc,
            ) {
                return crate::modules::fraction_init_with_vm(self, &new_args);
            }
            return func(&new_args);
        }

        // Calling a weak reference (`w()`) dereferences it: yields the
        // referent while alive, otherwise `None` — or the caller-supplied
        // default when one is passed (`w(default)`), matching CPython.
        if let PyObject::WeakRef { target, .. } = &*callable.borrow() {
            return Ok(match target.upgrade() {
                Some(rc) => PyObjectRef::Mut(rc),
                None => {
                    if let Some(default) = args.first() {
                        default.clone()
                    } else {
                        crate::object::py_none()
                    }
                }
            });
        }
        // Callable proxy: proxy is callable if target is callable
        if let PyObject::WeakProxy { target, .. } = &*callable.borrow() {
            if let Some(rc) = target.upgrade() {
                let target_ref = PyObjectRef::Mut(rc);
                return self.call_function(target_ref, args.to_vec(), keywords);
            } else {
                return Err(PyError::reference_error("weakly-referenced object no longer exists"));
            }
        }

        if let PyObject::BoundMethod { func, self_obj } = &*callable.borrow() {
            let func = func.clone();
            let self_obj = self_obj.clone();
            let mut new_args = vec![self_obj];
            new_args.extend(args);
            return self.call_function(func, new_args, keywords);
        }

        if let PyObject::Partial {
            func,
            args: partial_args,
            ..
        } = &*callable.borrow()
        {
            let func = func.clone();
            let mut all_args = partial_args.clone();
            all_args.extend(args);
            return self.call_function(func, all_args, keywords);
        }

        if let Some(result) = self.handle_py_function_call(&callable, args.clone(), keywords.clone()) {
            return result;
        }

        // `type` itself is special: calling it means `type(x)` (introspect)
        // or `type(name, bases, ns)` (metaclass-style construction), NOT
        // "instantiate the class named type" — must be checked by identity
        // before the generic Type-calling convention below, which would
        // otherwise build a plain Instance (wrong: `type(x)` must return a
        // real class/type object, not an instance of `type`).
        if self
            .builtins
            .get(&interner::intern("type"))
            .map(|t| t.is(&callable))
            .unwrap_or(false)
        {
            if !keywords.is_empty() {
                return Err(PyError::type_error("type() takes no keyword arguments"));
            }
            if args.len() == 1 {
                // type(obj) -> obj's real type. For a plain instance/value
                // this is its class (unchanged from before); for a class
                // object itself, it's that class's *metaclass* — plain
                // `type` unless something built it with a custom one (see
                // METATYPE_KEY) — never the class itself, which is what
                // the old, metaclass-unaware fallback incorrectly returned.
                let is_type_obj = matches!(&*args[0].borrow(), PyObject::Type { .. });
                if is_type_obj {
                    let mt = crate::object::metatype_of(&args[0]);
                    return Ok(mt.unwrap_or_else(|| callable.clone()));
                }
                return crate::object::builtin_type_of(&args);
            }
            if args.len() == 3 {
                // type(name, bases, ns) -> dynamic class creation. Uses
                // `self` directly (not `builtin_type_of`'s own 3-arg path,
                // which goes through `with_vm_mut` — safe when called with
                // no VM already active, but we're already inside one here
                // and a second aliasing `&mut VirtualMachine` from within
                // this same call chain reliably segfaulted in testing).
                let bases_vec = match &*args[1].borrow() {
                    PyObject::Tuple(t) => t.clone(),
                    PyObject::None => vec![],
                    _ => vec![args[1].clone()],
                };
                let namespace_dict = crate::object::dict_arg_to_hashmap(
                    &args[2],
                    "type() third argument must be a dict",
                )?;
                return self.default_build_class(
                    args[0].str(),
                    bases_vec,
                    namespace_dict,
                    vec![],
                    None,
                );
            }
            return crate::object::builtin_type_of(&args);
        }

        if let Some(result) = self.handle_metaclass_call(&callable, &args, &keywords) {
            return result;
        }

        // A class built by a custom metaclass that itself defines
        // `__call__` (e.g. enum's `EnumType.__call__`, used for `Color(2)`
        // value-lookup instead of construction) must dispatch through that
        // `__call__` — real Python semantics: `SomeClass(...)` really means
        // `type(SomeClass).__call__(SomeClass, ...)`, and only the
        // *default* `type.__call__` behaves like "construct + __init__",
        // which is what the generic Type-calling convention below
        // hardwires. Checked by identity-of-behavior (does the metatype
        // have its OWN `__call__`, distinct from `object`'s absence of
        // one) rather than unconditionally, so plain classes with no
        // custom metaclass are completely unaffected.
        {
            let mt = crate::object::metatype_of(&callable);
            if let Some(mt) = mt {
                if let Some(call_fn) = crate::object::lookup_dunder_via_mro(&mt, "__call__") {
                    let unwrapped = if let PyObject::StaticMethod { func } = &*call_fn.borrow() {
                        Some(func.clone())
                    } else {
                        None
                    };
                    let call_fn = unwrapped.unwrap_or(call_fn);
                    let mut call_args = vec![callable.clone()];
                    call_args.extend(args);
                    return self.call_function(call_fn, call_args, keywords);
                }
            }
        }
        // deque's iterators ARE constructible with a deque argument
        // (test_deque::test_reversed_new: klass = type(reversed(deque())) ;
        // list(klass(deque(s))) == list(reversed(s))). Handle before the
        // generic placeholder check below.
        if let PyObject::Type { name, .. } = &*callable.borrow() {
            if name == "deque_iterator" || name == "deque_reverse_iterator" {
                if args.len() != 1 {
                    return Err(PyError::type_error(format!(
                        "{} expected exactly one argument",
                        name
                    )));
                }
                let deque_obj = &args[0];
                let is_deque = if matches!(&*deque_obj.borrow(), PyObject::Deque { .. }) {
                    true
                } else if let Some(n) = crate::object::native_backing_of(deque_obj) {
                    matches!(&*n.borrow(), PyObject::Deque { .. })
                } else {
                    false
                };
                if !is_deque {
                    return Err(PyError::type_error(format!(
                        "{}() argument must be deque",
                        name
                    )));
                }
                let native = crate::object::native_backing_of(deque_obj)
                    .unwrap_or_else(|| deque_obj.clone());
                let len = if let PyObject::Deque { data, .. } = &*native.borrow() {
                    data.len()
                } else {
                    0
                };
                if name == "deque_iterator" {
                    return Ok(PyObjectRef::new(PyObject::DequeIter {
                        deque: deque_obj.clone(),
                        index: 0,
                        start_len: len,
                    }));
                } else {
                    let idx = if len == 0 { -1 } else { (len as isize) - 1 };
                    return Ok(PyObjectRef::new(PyObject::DequeRevIter {
                        deque: deque_obj.clone(),
                        index: idx,
                        start_len: len,
                    }));
                }
            }
        }
        // Placeholder iterator types (range_iterator, etc.) are not directly constructible
        if let PyObject::Type { name, .. } = &*callable.borrow() {
            if name.contains("iterator") || name == "select.poll" || name == "select.devpoll" || name == "select.epoll" || name == "select.kqueue" {
                return Err(PyError::type_error(format!(
                    "cannot create '{}' instances",
                    name
                )));
            }
        }

        // A real native value type (`int`, and eventually more — see
        // `NATIVE_VALUE_CTOR_KEY`'s doc comment) called DIRECTLY (`int(5)`,
        // as opposed to a user subclass like `class MyInt(int): ...`,
        // which still needs the Instance-building path below) dispatches
        // straight to its original native constructor and returns that
        // raw, UNWRAPPED result — never a `PyObject::Instance`. Checked
        // before `type_construct_info` so it takes priority over the
        // generic construction convention entirely.
        {
            let native_ctor = if let PyObject::Type { dict, .. } = &*callable.borrow() {
                dict.get_str(crate::object::NATIVE_VALUE_CTOR_KEY).cloned()
            } else {
                None
            };
            if let Some(ctor) = native_ctor {
                // `float()` takes positional-only args (`float(x='3.14')` is
                // `TypeError: float() takes no keyword arguments`, not a
                // "not 'dict'" error from the packed kwargs dict it would
                // otherwise arrive as — test_float's test_keyword_args).
                let is_float =
                    matches!(&*callable.borrow(), PyObject::Type { name, .. } if name == "float");
                if is_float && !keywords.is_empty() {
                    return Err(PyError::type_error("float() takes no keyword arguments"));
                }
                return self.call_function(ctor, args, keywords);
            }
        }

        if let Some(result) = self.handle_type_call(&callable, args.clone(), keywords.clone()) {
            return result;
        }

        if let PyObject::BuildClass = &*callable.borrow() {
            return self.handle_build_class(args, keywords);
        }

        // `staticmethod` objects are directly callable since Python 3.10
        // (they forward to the wrapped callable) — test_decorators asserts
        // `staticmethod(f)(1) == 1`. `classmethod` objects are NOT callable
        // (assertRaises(TypeError, classmethod(f), 1)), so no arm for those.
        if let PyObject::StaticMethod { func } = &*callable.borrow() {
            return self.call_function(func.clone(), args, keywords);
        }

        if let PyObject::Closure(c) = &*callable.borrow() {
            // Same "pack keywords into a trailing dict" convention as
            // `BuiltinFunction` just above — this early-return skipped it
            // entirely, so a `Closure`-implemented method called with
            // keyword arguments (real trigger: `namedtuple`'s own
            // `_replace(field=val)`) silently ran as if NO keywords were
            // passed at all (the override never took effect).
            if !keywords.is_empty() {
                let mut dict = crate::object::PyDict::new();
                for (k, v) in &keywords {
                    let _ = dict.set(crate::object::py_str(k), v.clone());
                }
                let mut new_args = args;
                new_args.push(crate::object::PyObjectRef::new(
                    crate::object::PyObject::Dict(Box::new(dict)),
                ));
                return c(&new_args);
            }
            return c(&args);
        }

        let call_dunder = {
            let borrowed = callable.borrow();
            if let PyObject::Instance { typ, .. } = &*borrowed {
                crate::object::lookup_dunder_via_mro(typ, "__call__")
            } else {
                None
            }
        };
        if let Some(f) = call_dunder {
            // `callable` must not still be borrowed here — if `__call__`'s
            // own body mutates `self` (e.g. `self.hits += 1`, common for a
            // caching wrapper), STORE_ATTR's borrow_mut() on the very same
            // object would otherwise panic with a RefCell conflict.
            //
            // When the found `__call__` is ITSELF an `Instance` (not a
            // `Function`/`BuiltinFunction`), this recurses straight back
            // into `self.call_function` via plain Rust call-stack recursion
            // — no Python frame is ever pushed for this step, so it never
            // passes through the `PyObject::Function` arm's own
            // `self.frames.len() >= self.recursion_limit` check at all. A
            // cyclic `__call__` chain (real trigger: CPython's own
            // `test_descr.py`'s `test_recursive_call` — `A.__call__ = A()`,
            // then `A()()`) previously recursed forever and overflowed the
            // real native stack instead of raising a catchable
            // `RecursionError`. Guarded the same way as the other
            // "disposable-VM-shaped" native recursion gap this session (see
            // `NativeDispatchRecursionGuard`'s own doc comment).
            let _guard = crate::object::NativeDispatchRecursionGuard::enter()?;
            let mut call_args = vec![callable.clone()];
            call_args.extend(args.iter().cloned());
            return self.call_function(f, call_args, keywords);
        }

        Err(PyError::type_error(format!(
            "'{}' object is not callable",
            type_name
        )))
    }

}



