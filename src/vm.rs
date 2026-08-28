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

thread_local! {
    static ATTR_CACHE: std::cell::RefCell<HashMap<(String, String), crate::object::BuiltinFunc>> = std::cell::RefCell::new(HashMap::new());
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

        // Native os module
        let os_mod = create_module("os", create_os_dict());
        // Add os.PathLike (PEP 519) — virtual subclass via __fspath__
        {
            use std::collections::HashMap as OsMap;
            let mut d: OsMap<String, PyObjectRef> = OsMap::new();
            // __abstractmethods__ = frozenset({'__fspath__'})
            {
                let mut s = crate::object::PySet::new();
                let _ = s.add(py_str("__fspath__"));
                d.insert(
                    "__abstractmethods__".to_string(),
                    PyObjectRef::new(PyObject::FrozenSet(s)),
                );
            }
            d.insert(
                "__fspath__".to_string(),
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: "__fspath__".to_string(),
                    func: |_args: &[PyObjectRef]| {
                        Err(PyError::type_error(
                            "PathLike.__fspath__() not implemented".to_string(),
                        ))
                    },
                }),
            );
            d.insert(
                "__instancecheck__".to_string(),
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: "__instancecheck__".to_string(),
                    func: |args: &[PyObjectRef]| {
                        if args.len() < 2 {
                            return Ok(crate::object::py_bool(false));
                        }
                        let has = args[1].borrow().get_attribute("__fspath__").is_ok();
                        Ok(crate::object::py_bool(has))
                    },
                }),
            );
            d.insert(
                "__subclasscheck__".to_string(),
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: "__subclasscheck__".to_string(),
                    func: |args: &[PyObjectRef]| {
                        if args.len() < 2 {
                            return Ok(crate::object::py_bool(false));
                        }
                        let sub = args[1].clone();
                        let has = crate::object::lookup_dunder_via_mro(&sub, "__fspath__").is_some()
                            || sub.borrow().get_attribute("__fspath__").is_ok();
                        Ok(crate::object::py_bool(has))
                    },
                }),
            );
            d.insert(
                "__class_getitem__".to_string(),
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: "__class_getitem__".to_string(),
                    func: |args: &[PyObjectRef]| {
                        if args.len() < 2 {
                            return Err(PyError::type_error(
                                "__class_getitem__() takes exactly 1 argument".to_string(),
                            ));
                        }
                        let origin = args[0].clone();
                        let item = args[1].clone();
                        let ga_type = crate::modules::get_generic_alias_type();
                        // GenericAlias instance stores origin and args
                        let mut ga_dict = std::collections::HashMap::new();
                        ga_dict.insert(
                            crate::interner::intern("__origin__"),
                            origin.clone(),
                        );
                        let tup = if let PyObject::Tuple(v) = &*item.borrow() {
                            item.clone()
                        } else {
                            crate::object::py_tuple(vec![item.clone()])
                        };
                        // Use the generic alias storage via instance dict is not enough;
                        // construct via the helper that creates a proper GenericAlias object.
                        // Fallback: create instance of GenericAlias type with _origin/_args
                        let ga = PyObjectRef::new(PyObject::Instance {
                            typ: ga_type.clone(),
                            dict: {
                                let mut m = crate::object::AttrMap::new();
                                m.insert("__origin__".to_string(), origin);
                                m.insert("__args__".to_string(), tup);
                                m
                            },
                        });
                        Ok(ga)
                    },
                }),
            );
            let pathlike_type = PyObjectRef::new(PyObject::Type {
                name: "PathLike".to_string(),
                dict: Box::new(crate::object::str_map_to_typedict(d)),
                bases: vec![],
                mro: vec![],
            });
            if let PyObject::Type { mro, .. } = &mut *pathlike_type.borrow_mut() {
                *mro = vec![pathlike_type.clone()];
            }
            if let PyObject::Module { dict, .. } = &mut *os_mod.borrow_mut() {
                dict.insert_str("PathLike", pathlike_type.clone());
            }
        }
        modules.insert_str("os", os_mod.clone());
        // posix is the C extension behind os — alias it for importlib compatibility
        modules.insert_str("posix", os_mod.clone());

        // Native os.path submodule (path manipulation functions)
        let os_path_mod = create_module("os.path", create_os_path_dict());
        // Wire path as a submodule attribute of the os parent module
        if let PyObject::Module { dict, .. } = &mut *os_mod.borrow_mut() {
            dict.insert_str("path", os_path_mod.clone());
        }
        modules.insert_str("os.path", os_path_mod.clone());
        // posixpath is the real module behind os.path on POSIX (CPython's
        // own os.py does `sys.modules['os.path'] = posixpath`) — code that
        // imports it directly (`import posixpath`, common in stdlib-ish
        // path-handling helpers) expects the same functions os.path has.
        modules.insert_str("posixpath", os_path_mod);

        let pathlib_dict = create_pathlib_dict();
        modules.insert_str("pathlib", create_module("pathlib", pathlib_dict));

        // Native urllib package (urllib.request, urllib.parse)
        let urllib_dict = create_urllib_dict();
        modules.insert_str("urllib", create_module("urllib", urllib_dict));

        let json_dict = create_json_dict();
        modules.insert_str("json", create_module("json", json_dict));

        let collections_dict = create_collections_dict(
            builtins
                .get(&interner::intern("object"))
                .cloned()
                .unwrap_or_else(py_none),
        );
        modules.insert_str(
            "collections",
            create_module("collections", collections_dict),
        );

        let functools_dict = create_functools_dict();
        modules.insert_str("functools", create_module("functools", functools_dict));

        let itertools_dict = create_itertools_dict();
        modules.insert_str("itertools", create_module("itertools", itertools_dict));

        let datetime_dict = create_datetime_dict();
        modules.insert_str("datetime", create_module("datetime", datetime_dict));
        // `_datetime` is real CPython's C-accelerated backing module —
        // `datetime.py` itself does `from _datetime import *` when
        // available. This interpreter's `datetime` is already a single,
        // natively-implemented module (no separate accelerated/pure-
        // Python split), so `_datetime` is just an alias — needed only so
        // code that imports `_datetime` directly (real trigger: CPython's
        // own `test_module.py`-style introspection, checking that both
        // names resolve) doesn't raise `ImportError`.
        modules.insert_str(
            "_datetime",
            create_module("_datetime", create_datetime_dict()),
        );
        // `_pydatetime` is real CPython's pure-Python datetime backing
        // module — aliased here exactly like `_datetime` (this interpreter's
        // datetime is a single native module), so test.support's
        // import_fresh_module-based harnesses and `test_datetime.py`'s
        // setUpClass (which reads `module._pydatetime`) work.
        modules.insert_str(
            "_pydatetime",
            create_module("_pydatetime", create_datetime_dict()),
        );

        let zoneinfo_dict = create_zoneinfo_dict();
        modules.insert_str("zoneinfo", create_module("zoneinfo", zoneinfo_dict));

        let socket_dict = create_socket_dict();
        modules.insert_str("socket", create_module("socket", socket_dict.clone()));
        modules.insert_str("_socket", create_module("_socket", socket_dict));

        let select_dict = create_select_dict();
        modules.insert_str("select", create_module("select", select_dict));

        let re_dict = create_re_dict();
        modules.insert_str("re", create_module("re", re_dict));

        let subprocess_dict = create_subprocess_dict();
        modules.insert_str("subprocess", create_module("subprocess", subprocess_dict));

        // Native pickle module (basic stub)
        modules.insert_str("_pickle", create_module("_pickle", create_pickle_dict()));

        // Native logging module
        modules.insert_str("_logging", create_module("_logging", create_logging_dict()));
        modules.insert_str(
            "_logging.config",
            create_module("_logging.config", create_logging_config_dict()),
        );

        // Native timeit module
        modules.insert_str("_timeit", create_module("_timeit", create_timeit_dict()));

        let threading_dict = create_threading_dict();
        modules.insert_str("threading", create_module("threading", threading_dict));

        // Native _thread module (CPython C extension replacement)
        modules.insert_str(
            "_thread",
            create_module("_thread", create_thread_module_dict()),
        );

        // Native signal module (CPython C extension replacement)
        modules.insert_str("signal", create_module("signal", create_signal_dict()));

        // Native gc module (CPython C extension replacement)
        modules.insert_str("gc", create_module("gc", create_gc_dict()));

        // Native sysconfig module (CPython stdlib replacement)
        modules.insert_str(
            "sysconfig",
            create_module("sysconfig", create_sysconfig_dict()),
        );

        // linecache: using Lib/linecache.py (native stub removed)

        // calendar: using Lib/calendar.py (pure Python, full CPython compat)

        // Native locale module
        modules.insert_str("locale", create_module("locale", create_locale_dict()));
        // Native _locale module — in real CPython this IS the C extension
        // that the pure-Python `locale` module delegates to. Registering it
        // natively (instead of the Lib/_locale.py stub) makes `setlocale`,
        // `localeconv`, etc. real shared state (see create_locale_dict).
        modules.insert_str("_locale", create_module("_locale", create_locale_dict()));

        // gettext module (mostly Python source — see install_source_defined_stdlib below)
        modules.insert_str("gettext", create_module("gettext", create_gettext_dict()));

        // Native ssl module (CPython C extension replacement for urllib3 compatibility)
        modules.insert_str("ssl", create_module("ssl", create_ssl_dict()));

        // Native time module
        modules.insert_str("time", create_module("time", create_time_dict()));

        // Native C extension replacements for CPython stdlib compatibility
        let weakref_dict = create_weakref_dict();
        modules.insert_str("_weakref", create_module("_weakref", weakref_dict.clone()));

        let collections_abc_dict = create_collections_abc_dict();
        modules.insert_str(
            "_collections_abc",
            create_module("_collections_abc", collections_abc_dict.clone()),
        );
        // Pre-register collections.abc so the import chain walker finds it without needing __path__
        modules.insert_str(
            "collections.abc",
            create_module("collections.abc", collections_abc_dict),
        );

        // Native weakref module DISABLED: use Lib/weakref.py (needs only _weakref primitives)
        // let mut weakref_mod_dict = weakref_dict; // Start from _weakref
        // weakref_mod_dict.insert_str("WeakValueDictionary", create_weakref_weak_val_dict());
        // weakref_mod_dict.insert_str("WeakKeyDictionary", create_weakref_weak_key_dict());
        // weakref_mod_dict.insert_str("WeakSet", create_weakref_weak_set());
        // modules.insert_str("weakref", create_module("weakref", weakref_mod_dict));

        // Native copy module DISABLED: use Lib/copy.py
        // modules.insert_str("copy", create_module("copy", create_copy_dict()));

        // Native types module (replaces CPython types.py)
        modules.insert_str(
            "_types_native",
            create_module("_types_native", create_types_dict()),
        );

        // Native struct module for binary packing
        modules.insert_str("struct", create_module("struct", create_struct_dict()));

        // Native bisect module for binary search
        modules.insert_str("bisect", create_module("bisect", create_bisect_dict()));
        // `_bisect` — real CPython's C accelerator for `bisect`; CPython's
        // own `test_bisect.py` builds its C-backed test class via
        // `import_fresh_module('bisect', fresh=['_bisect'])`, which failed
        // with `ModuleNotFoundError: No module named '_bisect'`, yielding
        // `module = None` and `'NoneType' object has no attribute
        // 'bisect_right'` for every C-class test. Same dict as `bisect`
        // (this interpreter has no separate pure-Python wrapper).
        modules.insert_str("_bisect", create_module("_bisect", create_bisect_dict()));

        // Native heapq module for heap queue operations
        // Native heapq DISABLED: use Lib/heapq.py (CPython 3.14) for full functionality
        // modules.insert_str("heapq", create_module("heapq", create_heapq_dict()));

        // enum module — real Enum/IntEnum/StrEnum/EnumType semantics
        // (metaclass, real members, auto/unique) are far easier and more
        // correct expressed as real Python source (see enum_extra.py)
        // than as hand-written Rust closures; install_source_defined_stdlib
        // (called below, once builtins/type registry exist) fills this
        // module's dict in. The empty dict here is just a placeholder
        // registration so that call finds an existing module to populate.
        modules.insert_str("enum", create_module("enum", HashMap::new()));

        // Native glob module
        modules.insert_str("glob", create_module("glob", create_glob_dict()));

        // Native fnmatch module
        modules.insert_str("fnmatch", create_module("fnmatch", create_fnmatch_dict()));

        // Native textwrap module DISABLED: use Lib/textwrap.py (CPython 3.14)
        // The native implementation was incomplete (simple whitespace
        // splitting, missing hyphen handling, dedent/indent bugs, missing
        // _split, etc.) and shadowed the correct pure-Python stdlib.
        // modules.insert_str(
        //     "textwrap",
        //     create_module("textwrap", create_textwrap_dict()),
        // );

        // `pprint` and `reprlib` are loaded from the vendored real CPython
        // Lib/ modules (their class-based APIs — PrettyPrinter/Repr — are
        // what test_pprint/test_reprlib exercise, and the native Rust
        // versions only had the top-level helper functions).

        // Native hashlib module
        modules.insert_str("hashlib", create_module("hashlib", create_hashlib_dict()));
        // `_hashlib` is real CPython's C-accelerated hashlib backing module —
        // alias it so test_hmac.py's `from _hashlib import ...` works.
        modules.insert_str("_hashlib", create_module("_hashlib", create_hashlib_dict()));

        // Native secrets module
        modules.insert_str("secrets", create_module("secrets", create_secrets_dict()));

        // Native hmac module
        modules.insert_str("hmac", create_module("hmac", create_hmac_dict()));
        // `_hmac` is real CPython's C-accelerated hmac backing module —
        // alias it like `_datetime` so code importing it directly
        // (CPython's test_hmac.py setUpClass) doesn't raise ImportError.
        modules.insert_str("_hmac", create_module("_hmac", create_hmac_dict()));

        // Native base64 module
        modules.insert_str("base64", create_module("base64", create_base64_dict()));

        // Native binascii module
        modules.insert_str(
            "binascii",
            create_module("binascii", create_binascii_dict()),
        );

        // Native uuid module
        modules.insert_str("uuid", create_module("uuid", create_uuid_dict()));

        // Native string module (with capwords and Formatter)
        let mut string_dict = create_string_dict();
        let string_v2 = create_string_dict_v2();
        for (k, v) in string_v2 {
            string_dict.insert(k, v);
        }
        modules.insert_str("string", create_module("string", string_dict));

        // Native colorsys module
        modules.insert_str(
            "colorsys",
            create_module("colorsys", create_colorsys_dict()),
        );

        // Native wave module
        modules.insert_str("wave", create_module("wave", create_wave_dict()));

        // Native numbers module — DISABLED: was bare STRING placeholders
        // (`d.insert_str("Number", py_str("Number"))` etc.) instead of
        // real ABC classes — `isinstance(x, numbers.Number)` may have
        // worked via some ad hoc string-matching path, but anything
        // doing real class things with them (`numbers.Number.register
        // (Decimal)` — needed by `decimal`'s own real implementation,
        // see its own doc comment above) raised `AttributeError: 'str'
        // object has no attribute 'register'`. Real CPython's `numbers.
        // py` is small (427 lines) and pure Python (just `abc.ABCMeta`/
        // `abstractmethod`) — vendored verbatim instead, same pattern as
        // `decimal`/`html.parser` above. `create_numbers_dict` (modules/
        // misc.rs) is now dead code, kept only in case `Lib/numbers.py`
        // needs to be reverted.
        // modules.insert_str("numbers", create_module("numbers", create_numbers_dict()));

        // `ast` now loads from Lib/ast.py — needs real (if minimal, marker-
        // only) node classes for PEP 649 lazy-annotation stringification
        // (`annotationlib.py`'s `_Stringifier`, needed transitively by
        // `test.support`), which the old native stub (just `literal_eval`
        // plus a handful of node NAMES as bare strings, no real classes at
        // all) couldn't provide. The old stub's actual `literal_eval`
        // logic is kept and re-exposed under a private native module name
        // so Lib/ast.py can still delegate to it instead of reimplementing
        // literal parsing in pure Python.
        modules.insert_str(
            "_ast_native",
            create_module("_ast_native", create_ast_dict()),
        );

        // Native sunau module (Sun AU audio format stubs)
        modules.insert_str("sunau", create_module("sunau", create_sunau_dict()));

        // Native csv module
        let csv_dict = create_csv_dict();
        let csv_mod = create_module("csv", csv_dict.clone());
        let csv_mod2 = create_module("_csv", csv_dict);
        modules.insert_str("csv", csv_mod.clone());
        modules.insert_str("_csv", csv_mod2);

        // Native io module — DISABLED: CPython io.py is used instead (imports from _io)
        // modules.insert_str("io", create_module("io", create_io_dict()));

        // Native statistics module. Tried vendoring the real CPython
        // `Lib/statistics.py` (same pattern as `html`/`numbers`/etc.) —
        // reverted: it hits the same unresolved "native types aren't real
        // Type objects" architecture gap as `decimal` (`type(5) is int`
        // is `False` here, breaking `_coerce`'s `T is S`/`T is int`
        // identity checks throughout `_sum`/`variance`/etc.), AND
        // `test_statistics.py` timed out (60s+) rather than completing —
        // an operational risk not worth taking for a module that would
        // still fail most of its own tests anyway. Several genuinely
        // general bugs found chasing this ARE kept (not reverted):
        // `functools.reduce`'s dropped-initial-value bug, `math.fsum`
        // only handling List/Tuple, `itertools.groupby` (was missing
        // entirely), `int.as_integer_ratio`/`numerator`/`denominator`/
        // `real`/`imag`, `call_bound_method` supporting `type` as a
        // plain callable, and — the most impactful one — `WITH_EXIT`
        // never recognizing a user-defined exception CLASS instance
        // (only the native `PyObject::Exception` shape), which broke
        // `with`-statement `__exit__(exc_type, exc_value, tb)` for any
        // custom exception (`unittest.assertRaises` reported ANY custom
        // exception as "not raised" even when it genuinely was).
        // Native statistics module DISABLED: real Lib/statistics.py is used instead
        // (the native implementation only had mean/median/stdev/harmonic_mean/mode/
        //  median_low/median_high — missing variance, pvariance, pstdev,
        //  median_grouped, geometric_mean, NormalDist, fmean, quantiles, etc.)
        // modules.insert_str(
        //     "statistics",
        //     create_module("statistics", create_statistics_dict()),
        // );

        // Native contextlib module — DISABLED: real Lib/contextlib.py is used instead
        // modules.insert_str("contextlib", create_module("contextlib", create_contextlib_dict()));

        // Native decimal module. (Attempted vendoring real CPython's
        // `decimal.py`/`_pydecimal.py` this session — got as far as
        // `numbers.Number.register(Decimal)` working [needed the
        // `numbers` vendor + a new generic `.register()`/`isinstance`-
        // registry fallback, both kept, see below] before hitting
        // `int.bit_length` accessed UNBOUND off the `int` type itself
        // (`_nbits = int.bit_length`) — `int` is a `PyObject::
        // BuiltinFunction`, not a real `Type`, so it has no attribute
        // lookup for "what method would an instance's `.bit_length()`
        // resolve to" at all. That's a deeper, general "unbound method
        // access on a native-backed type" gap shared with list/dict/str/
        // etc., not specific to decimal — reverted the vendor rather
        // than chase it further this session. `Lib/decimal.py`/`Lib/
        // _pydecimal.py` were removed again; re-attempt once unbound
        // native-type method access is fixed.)
        modules.insert_str("decimal", create_module("decimal", create_decimal_dict()));

        // Native fractions module
        modules.insert_str(
            "fractions",
            create_module("fractions", create_fractions_dict()),
        );

        // Native platform module
        modules.insert_str(
            "platform",
            create_module("platform", create_platform_dict()),
        );

        // `getopt` — the real CPython Lib/getopt.py (vendored) is loaded
        // from disk instead: it exposes the internals CPython's own
        // test_getopt.py exercises directly (do_shorts/do_longs/
        // gnu_getopt/GetoptError/short_has_arg/long_has_args) that the
        // native Rust version never had, and its algorithm is authoritative.

        // Native getpass module
        modules.insert_str("getpass", create_module("getpass", create_getpass_dict()));

        // Native errno module
        modules.insert_str("errno", create_module("errno", create_errno_dict()));

        // Native _random module (C extension stub for CPython's random.py)
        modules.insert_str(
            "_random",
            create_module("_random", create_random_cmodule_dict()),
        );

        // Native shutil module
        modules.insert_str("shutil", create_module("shutil", create_shutil_dict()));

        // Native graphlib module
        modules.insert_str(
            "graphlib",
            create_module("graphlib", create_graphlib_dict()),
        );

        // Native pdb module
        modules.insert_str("pdb", create_module("pdb", create_pdb_dict()));

        // traceback now loads from Lib/traceback.py — the old native stub
        // (`create_traceback_dict`, kept as dead code) had only
        // `format_exc`/`print_exc` as no-ops and no `TracebackException`
        // at all, which real `unittest/result.py` needs to format a
        // failure/error for display.
        // modules.insert_str("traceback", create_module("traceback", create_traceback_dict()));

        // Native warnings module
        modules.insert_str(
            "warnings",
            create_module("warnings", create_warnings_dict()),
        );

        // Native abc module
        modules.insert_str("abc", create_module("abc", create_abc_dict()));

        // `_py_abc` — real CPython's separate pure-Python reference
        // implementation of `ABCMeta` (used by `test_abc.py`'s own
        // `test_factory(abc.ABCMeta, ...)` / `test_factory(_py_abc.ABCMeta,
        // ...)` pattern to run its whole suite against both the C and
        // Python implementations). This codebase has only ONE `ABCMeta`
        // implementation (native Rust, no separate "C vs Python" split at
        // all), so `_py_abc` was missing entirely — `import _py_abc`
        // failed outright, crashing `test_abc.py` at collection before a
        // single test ran. Aliased to the exact same dict as `abc` itself:
        // not a literal from-scratch Python reimplementation, but an
        // honest match for this codebase's actual architecture — it
        // unblocks the import and lets both `test_factory` calls exercise
        // real, working `ABCMeta` functionality (just the same
        // implementation twice under two names, rather than two distinct
        // ones).
        modules.insert_str("_py_abc", create_module("_py_abc", create_abc_dict()));

        // Native typing module (type annotation stubs)
        // Comment out native typing - use Lib/typing.py instead
        // modules.insert_str("typing", create_module("typing", create_typing_dict()));

        // Native pickle module
        modules.insert_str("_pickle", create_module("_pickle", create_pickle_dict()));

        // Native logging module
        modules.insert_str("_logging", create_module("_logging", create_logging_dict()));

        // Native timeit module
        modules.insert_str("_timeit", create_module("_timeit", create_timeit_dict()));

        // Native json.tool module
        modules.insert_str(
            "json.tool",
            create_module("json.tool", create_json_tool_dict()),
        );

        // Native cmath module (complex math: sqrt, sin, cos)
        modules.insert_str("cmath", create_module("cmath", create_cmath_dict()));

        // Native gzip module
        modules.insert_str("gzip", create_module("gzip", create_gzip_dict()));

        // Native zlib module
        modules.insert_str("zlib", create_module("zlib", create_zlib_dict()));

        // Native tarfile module
        modules.insert_str("tarfile", create_module("tarfile", create_tarfile_dict()));

        // Native zipfile module (read-only)
        modules.insert_str("zipfile", create_module("zipfile", create_zipfile_dict()));

        // Native hashlib_extra module
        modules.insert_str(
            "hashlib_extra",
            create_module("hashlib_extra", create_hashlib_extra_dict()),
        );

        // dataclasses now loads from Lib/dataclasses.py (a real, if
        // simplified, implementation — field generation, generated
        // __init__/__repr__/__eq__, __dataclass_fields__, fields(), etc.)
        // instead of this native stub, which only ever tagged classes with
        // a marker attribute and never generated anything.
        // modules.insert_str("dataclasses", create_module("dataclasses", create_dataclasses_dict()));

        // Native operator module
        modules.insert_str(
            "operator",
            create_module("operator", create_operator_dict()),
        );
        // `_operator` — real CPython's C-accelerated backing module for
        // `operator.py` (`from _operator import *`); same alias rationale
        // as `_datetime` above.
        modules.insert_str(
            "_operator",
            create_module("_operator", create_operator_dict()),
        );

        // Native reprlib module — REMOVED (reprlib is now loaded from the
        // vendored real Lib/reprlib.py, which provides the Repr class).

        // Native array module
        modules.insert_str("array", create_module("array", create_array_dict()));

        // Native shelve module (persistent dict wrapper)
        modules.insert_str("shelve", create_module("shelve", create_shelve_dict()));
        modules.insert_str("selectors", create_module("selectors", create_selectors_dict()));

        // "mimetypes" intentionally NOT registered natively: the
        // pure-Python Lib/mimetypes.py is the full CPython module
        // (incl. _default_mime_types, init(), MimeTypes class); the old
        // native stub shadowed it and broke its own test suite at
        // setUpModule. Re-enable only if import-from-Lib breaks.

        // Native dis module for bytecode disassembly
        modules.insert_str("dis", create_module("dis", create_dis_dict()));

        // Native http module (HTTPStatus enum)
        let http_mod = create_module("http", create_http_dict());
        modules.insert_str("http", http_mod.clone());

        // Native http.client submodule (HTTPConnection, HTTPResponse)
        let http_client_mod = create_module("http.client", create_http_client_dict());
        // Wire client as a submodule attribute of the http parent module
        if let PyObject::Module { dict, .. } = &mut *http_mod.borrow_mut() {
            dict.insert_str("client", http_client_mod.clone());
        }
        modules.insert_str("http.client", http_client_mod);

        // Native smtplib module (SMTP stub)
        modules.insert_str("smtplib", create_module("smtplib", create_smtplib_dict()));

        // Native html/html.entities/html.parser — DISABLED: `html.parser`
        // was a near-empty stub (no real tokenizer at all — `feed()` just
        // accumulated raw text verbatim, none of `handle_starttag`/
        // `handle_endtag`/`handle_data`/etc. were ever called), and built
        // as a `PyObject::BuiltinFunction` rather than a real
        // `PyObject::Type`, so `class EventCollector(html.parser.
        // HTMLParser): ...` (real subclassing, overriding those handler
        // methods — CPython's own `test_htmlparser.py`'s entire approach)
        // couldn't inherit anything from it at all (`AttributeError:
        // 'EventCollector' object has no attribute 'feed'`). Real
        // CPython's `html`/`html.entities`/`html.parser` are pure Python
        // (plus `_markupbase`, `html.parser`'s shared tokenizer-support
        // base) — vendored verbatim from a real CPython 3.14 install
        // rather than reimplemented, same "vendor as pure-Python Lib/
        // module" pattern as `unittest`/`doctest`/`email` above. Resolved
        // through the normal file-based import path instead —
        // `Lib/html/__init__.py` (escape/unescape), `Lib/html/
        // entities.py` (html5/name2codepoint/codepoint2name data),
        // `Lib/html/parser.py` (the real HTMLParser), `Lib/
        // _markupbase.py`. `create_html_dict`/`create_html_entities_dict`/
        // `create_html_parser_dict` (modules/net.rs, modules/text.rs) are
        // now dead code, kept only in case `Lib/html/` needs to be
        // reverted.
        // let html_mod = create_module("html", create_html_dict());
        // modules.insert_str("html", html_mod.clone());
        // let html_entities_mod = create_module("html.entities", create_html_entities_dict());
        // if let PyObject::Module { dict, .. } = &mut *html_mod.borrow_mut() {
        //     dict.insert_str("entities", html_entities_mod.clone());
        // }
        // modules.insert_str("html.entities", html_entities_mod);
        // let html_parser_mod = create_module("html.parser", create_html_parser_dict());
        // if let PyObject::Module { dict, .. } = &mut *html_mod.borrow_mut() {
        //     dict.insert_str("parser", html_parser_mod.clone());
        // }
        // modules.insert_str("html.parser", html_parser_mod);

        // Native unittest module — DISABLED: was a complete no-op stub
        // (every assertX method silently did nothing, `main()` never
        // discovered or ran a single test) — replaced with the real
        // CPython pure-Python `unittest` package (Lib/unittest/). Real
        // CPython/Django test suites are unittest-based; silently
        // no-op'ing every assertion is actively dangerous for a project
        // whose goal is being a genuine CPython replacement.
        // modules.insert_str("unittest", create_module("unittest", create_unittest_dict()));

        // Native doctest module used to be a hollow stub (testmod/testfile
        // always reported 0 attempted/0 failed regardless of actual
        // docstring content; DocTestSuite/DocFileSuite didn't exist at all
        // — real trigger: 16+ CPython test files' own `load_tests` doing
        // `tests.addTest(doctest.DocTestSuite())`, crashing with
        // `AttributeError: 'module' object has no attribute
        // 'DocTestSuite'`). Replaced with a real (if simplified) Python
        // implementation at `Lib/doctest.py`, resolved through the normal
        // file-based import path instead — same "vendor/reimplement as a
        // pure-Python Lib/ module" pattern as `unittest`/`email`.
        // modules.insert_str("doctest", create_module("doctest", create_doctest_dict()));

        // `email` used to be a thin native stub (EmailMessage/MIMEText/
        // header/utils only, no real Message class, no submodule files —
        // couldn't satisfy `import email.message`/`email.mime.multipart`/
        // etc. at all). Real CPython's own `email` package is pure
        // Python and self-contained; a full copy now lives at
        // `Lib/email/` (plus `Lib/quopri.py`, one of its few deps) and is
        // resolved through the normal file-based import path instead —
        // no native registration needed anymore. `create_email_dict`/
        // `create_email_mime_text_dict`/`create_email_header_dict`/
        // `create_email_utils_dict` (modules/misc.rs) are now dead code,
        // kept only in case `Lib/email/` needs to be reverted.

        // Native configparser module
        // `configparser` is loaded from the vendored real CPython
        // Lib/configparser.py (the native Rust version was missing module
        // internals like `_default_dict` that test_configparser exercises).

        // Native xml.etree.ElementTree module
        let xml_etree_mod = create_module("xml.etree.ElementTree", create_xml_etree_dict());
        modules.insert_str("xml.etree.ElementTree", xml_etree_mod.clone());
        // `xml.etree` (the bare PACKAGE, distinct from its
        // `.ElementTree` submodule) previously had no entry of its own in
        // `vm.modules` at all — only the leaf `xml.etree.ElementTree` was
        // registered — so `import xml.etree` (without the submodule
        // suffix, a real, common form — real trigger: several CPython
        // corpus files) raised `ImportError: No module named 'xml.etree'`
        // even though the deeper `xml.etree.ElementTree` import worked
        // fine. Fixed by registering the package itself too, with
        // `ElementTree` wired as its own attribute (mirroring the
        // existing `xml`-package-wires-`etree` pattern just below).
        let xml_etree_pkg = create_module("xml.etree", HashMap::new());
        if let PyObject::Module {
            dict: xml_etree_pkg_dict,
            ..
        } = &mut *xml_etree_pkg.borrow_mut()
        {
            xml_etree_pkg_dict.insert_str("ElementTree", xml_etree_mod.clone());
        }
        modules.insert_str("xml.etree", xml_etree_pkg.clone());
        // Native xml module (empty package)
        let xml_mod = create_module("xml", create_xml_dict());
        // Wire etree as a submodule of xml
        if let PyObject::Module {
            dict: xml_el_dict, ..
        } = &mut *xml_mod.borrow_mut()
        {
            xml_el_dict.insert_str("etree", xml_etree_pkg.clone());
        }
        modules.insert_str("xml", xml_mod);

        // Native this module (Zen of Python)
        modules.insert_str("this", create_module("this", create_this_dict()));

        // argparse now loads from Lib/argparse.py (real CPython source,
        // vendored verbatim) instead of the old native stub — the stub's
        // `add_argument` was a no-op and `parse_args` never populated a
        // caller-supplied `namespace` object (2nd positional arg), which
        // is exactly the calling convention `unittest.main()`'s own
        // `TestProgram.parseArgs` and Django's management-command
        // machinery both rely on. See `create_argparse_dict` (kept, now
        // dead code) for the old implementation.
        // modules.insert_str("argparse", create_module("argparse", create_argparse_dict()));

        // Native _imp module (CPython C extension replacement needed by importlib._bootstrap)
        modules.insert_str("_imp", create_module("_imp", create_imp_dict()));
        // Native _opcode module (needed by test.support)
        modules.insert_str("_opcode", create_module("_opcode", create_opcode_dict()));
        // Native _warnings module (CPython C extension replacement)
        modules.insert_str(
            "_warnings",
            create_module("_warnings", create_warnings_c_dict()),
        );
        // Native marshal module (CPython C extension replacement)
        modules.insert_str("marshal", create_module("marshal", create_marshal_dict()));
        // Native zipimport module stub
        modules.insert_str(
            "zipimport",
            create_module("zipimport", create_zipimport_dict()),
        );
        // Native _io module (CPython C extension replacement needed by importlib._bootstrap_external)
        modules.insert_str("_io", create_module("_io", create_io_module_dict()));
        // Native queue module (Queue backed by PyObject::Queue)
        modules.insert_str("queue", create_module("queue", create_queue_dict()));

        // Native importlib stub module
        let importlib_mod = create_module("importlib", create_importlib_dict());
        // Wire importlib.resources as a submodule
        {
            let resources_mod =
                create_module("importlib.resources", create_importlib_resources_dict());
            if let PyObject::Module { dict, .. } = &mut *importlib_mod.borrow_mut() {
                dict.insert_str("resources", resources_mod.clone());
            }
            modules.insert_str("importlib.resources", resources_mod);
        }
        // Wire importlib.util as a submodule
        {
            let util_mod = create_module("importlib.util", create_importlib_util_dict());
            if let PyObject::Module { dict, .. } = &mut *importlib_mod.borrow_mut() {
                dict.insert_str("util", util_mod.clone());
            }
            modules.insert_str("importlib.util", util_mod);
        }
        // Add __path__ so dotted imports like importlib.machinery can find filesystem submodules
        {
            if let PyObject::Module { dict, .. } = &mut *importlib_mod.borrow_mut() {
                dict.insert_str(
                    "__path__",
                    py_list(vec![py_str(&format!("{}/importlib", find_lib_dir()))]),
                );
            }
        }
        modules.insert_str("importlib", importlib_mod);

        modules.insert_str("inspect", create_module("inspect", create_inspect_dict()));

        // Native __future__ module (needed by requests, etc.)
        modules.insert_str(
            "__future__",
            create_module("__future__", create_future_dict()),
        );

        // Native asyncio module (basic event loop)
        modules.insert_str("asyncio", create_module("asyncio", create_asyncio_dict()));

        // Native atexit module (register/unregister exit callbacks)
        modules.insert_str("atexit", create_module("atexit", create_atexit_dict()));

        // Native contextvars module (ContextVar with thread-local storage)
        modules.insert_str(
            "contextvars",
            create_module("contextvars", create_contextvars_dict()),
        );

        // Native unicodedata module (basic Unicode category/normalize)
        modules.insert_str(
            "unicodedata",
            create_module("unicodedata", create_unicodedata_dict()),
        );

        // Native profile module
        modules.insert_str("profile", create_module("profile", create_profile_dict()));

        // Native cProfile module
        modules.insert_str(
            "cProfile",
            create_module("cProfile", create_cprofile_dict()),
        );

        // Native resource module (POSIX resource usage stubs)
        modules.insert_str(
            "resource",
            create_module("resource", create_resource_dict()),
        );

        // Native trace module (code tracing / coverage stubs)
        modules.insert_str("trace", create_module("trace", create_trace_dict()));

        // Native _concurrent module (concurrent.futures backend)
        let concurrent_futures_mod =
            create_module("concurrent.futures", create_concurrent_futures_dict());
        // Create intermediate concurrent package and wire futures under it
        let concurrent_mod = create_module("concurrent", HashMap::new());
        {
            let mut conc_mut = concurrent_mod.borrow_mut();
            if let PyObject::Module { dict, .. } = &mut *conc_mut {
                dict.insert_str("futures", concurrent_futures_mod.clone());
            }
        }
        modules.insert_str("concurrent", concurrent_mod);
        modules.insert_str("concurrent.futures", concurrent_futures_mod);

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


    fn execute_instruction(&mut self) -> PyResult<Option<PyObjectRef>> {
        let fi = self.frames.len() - 1;
        let ip = self.frames[fi].ip;
        if ip >= self.frames[fi].code.instructions.len() {
            return Err(PyError::runtime_error("execution reached end of code"));
        }
        // `last_error_line`/`last_error_file` (only ever read once, in
        // `main.rs`'s final top-level uncaught-error report) used to be
        // updated HERE — unconditionally, on every single instruction
        // executed, including a `.clone()` of the filename string — instead
        // of only at the point an error actually escapes uncaught (set in
        // `execute_inner`'s error-handling branch below, right where the
        // equivalent `last_traceback` entry is already computed from the
        // exact same frame/line). For a hot loop or recursive function
        // executing millions of instructions, that was millions of
        // pointless heap allocations doing bookkeeping nothing ever reads
        // unless the program is about to crash.
        let op = self.frames[fi].code.instructions[ip].op;
        let arg = self.frames[fi].code.instructions[ip].arg;
        self.frames[fi].ip = ip + 1;
        // Env-gated opcode histogram (RPY_OPCODE_HIST=1): one relaxed atomic
        // load per instruction when enabled, nothing when the flag was never
        // set. Dumped from main.rs at exit. Purely a profiling aid.
        if OPCODE_HIST_ENABLED.load(std::sync::atomic::Ordering::Relaxed) {
            let slot = &OPCODE_HIST[(op as usize) % OPCODE_HIST.len()];
            slot.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        // Debug: print instruction (only with profile feature)
        if cfg!(feature = "profile") {
            if matches!(
                op,
                Opcode::LOAD_GLOBAL
                    | Opcode::LOAD_FAST
                    | Opcode::CALL
                    | Opcode::LOAD_ATTR
                    | Opcode::RETURN_VALUE
            ) {
                let _frame_name = &self.frames[fi].code.name;
            }
        }

        // Profile: increment counter for this instruction
        // Only in profile mode (disabled by default for speed)
        if cfg!(feature = "profile") {
            let func_id = fi; // use frame index as function identifier
            let mut prof = self.profile.borrow_mut();
            let counters = prof
                .entry(func_id)
                .or_insert_with(|| vec![0u32; self.frames[fi].code.instructions.len()]);
            if ip < counters.len() {
                counters[ip] = counters[ip].saturating_add(1);
            }
        }

        match op {
            Opcode::NOP => {}

            Opcode::LOAD_CONST => {
                let const_idx = arg as usize;
                // Fast path: this exact LOAD_CONST was already parsed once
                // before (by this same CodeObject, possibly on a PRIOR call
                // — the cache lives on the `Rc`-shared CodeObject itself,
                // not the per-call Frame) — every `consts[i]` value is a
                // pure, deterministic source of one Python value, so a
                // cached hit is always correct, never stale (see
                // `CodeObject::const_cache`'s own doc comment for why this
                // is safe unlike the Frame-level LOAD_ATTR/LOAD_GLOBAL
                // caches, which read genuinely mutable state).
                let cached = self.frames[fi]
                    .code
                    .const_cache
                    .borrow()
                    .get(const_idx)
                    .and_then(|c| c.clone());
                let obj = if let Some(obj) = cached {
                    obj
                } else {
                    let const_val = self.frames[fi]
                        .code
                        .consts
                        .get(const_idx)
                        .ok_or_else(|| {
                            PyError::runtime_error(format!(
                                "constant index out of range: {}",
                                const_idx
                            ))
                        })?
                        .clone();
                    let obj = eval_const_value(const_val)?;
                    let mut cache = self.frames[fi].code.const_cache.borrow_mut();
                    if cache.len() <= const_idx {
                        cache.resize(const_idx + 1, None);
                    }
                    cache[const_idx] = Some(obj.clone());
                    obj
                };
                self.frames[fi].push(obj);
            }

            Opcode::LOAD_NAME => {
                let name_idx = arg as usize;
                let name = crate::interner::lookup_str(self.frames[fi].code.names[name_idx]);
                let val = {
                    let f = &self.frames[self.frames.len() - 1];
                    f.get_local(name)
                        .cloned()
                        .or_else(|| {
                            // A class body's own namespace (checked via get_local
                            // above) takes priority, but if the name isn't defined
                            // there, it may still be a free variable closed over
                            // from an enclosing function — matching CPython's
                            // LOAD_CLASSDEREF fallback (class bodies skip enclosing
                            // *function* scopes when resolving names normally, but
                            // methods defined inside still need to close over them,
                            // so this frame's own code object carries them as
                            // freevars/a closure exactly like a nested function).
                            let fv_idx = f.code.freevars.iter().position(|n| n == name)?;
                            let cell = f.closure.get(fv_idx)?;
                            match &*cell.borrow() {
                                PyObject::Cell { value: Some(inner) } => Some(inner.clone()),
                                PyObject::Cell { value: None } => None,
                                _ => Some(cell.clone()),
                            }
                        })
                        .or_else(|| f.globals.borrow().get(&interner::intern(name)).cloned())
                        .or_else(|| {
                            // Check module_globals (enclosing module scope for class bodies)
                            f.module_globals
                                .as_ref()
                                .and_then(|mg| mg.borrow().get(&interner::intern(name)).cloned())
                        })
                        .or_else(|| f.builtins.get(&interner::intern(name)).cloned())
                };
                match val {
                    Some(v) => self.frames[fi].push(v),
                    None => return Err(PyError::name_error_for(name)),
                }
            }

            Opcode::STORE_NAME => {
                let name_idx = arg as usize;
                let name = crate::interner::lookup(self.frames[fi].code.names[name_idx]);
                let val = self.frames[fi].pop()?;
                if let Some(order) = self.frames[fi].name_order.clone() {
                    let mut order = order.borrow_mut();
                    if !order.contains(&name) {
                        order.push(name.clone());
                    }
                }
                if let Some(live_module) = self.frames[fi].live_module.clone() {
                    if let PyObject::Module { dict, .. } = &mut *live_module.borrow_mut() {
                        dict.insert_str(&name, val.clone());
                    }
                }
                let sid = interner::intern(&name);
                self.frames[fi].globals.borrow_mut().insert(sid, val.clone());
                // Mirror to module.__dict__ for function frames where
                // live_module is None (e.g. `global __cached...` inside
                // `interpreter_requires_environment`). `STORE_NAME` is
                // used for that global (not STORE_GLOBAL) in this
                // codebase's compiler, so without this the cache update
                // stays only in `frame.globals` (the Rc captured by the
                // function) and is invisible via `module.__dict__`, so
                // `script_helper.__dict__['__cached...'] = None` resets
                // in tests never actually clear the function's view.
                // Do NOT mirror for class-body frames (name_order.is_some()):
                // class attributes like `type = ...` or `pprint = ...`
                // would otherwise leak into the enclosing module's globals
                // and shadow builtins/module-level names (e.g. `type`,
                // `pprint` in `_colorize`/`pprint` modules).
                if self.frames[fi].name_order.is_none() {
                    let mod_name_opt = self.frames[fi]
                        .globals
                        .borrow()
                        .get(&interner::intern("__name__"))
                        .cloned();
                    if let Some(mod_name_ref) = mod_name_opt {
                        if let PyObject::Str(s) = &*mod_name_ref.borrow() {
                            if let Some(mod_ref) = self.modules.get(s.as_str()).cloned() {
                                if let PyObject::Module { dict, .. } = &mut *mod_ref.borrow_mut() {
                                    dict.insert(sid, val.clone());
                                }
                            }
                        }
                    }
                    if let Some(mg) = self.frames[fi].module_globals.clone() {
                        mg.borrow_mut().insert(sid, val);
                    } else if self.frames[fi].globals.borrow().contains_key(&sid) {
                        // already inserted above
                    }
                }
                if self.frames[fi].frame_locals_obj.is_some() {
                    self.sync_frame_locals(fi);
                }
            }

             Opcode::LOAD_FAST => {
                let var_idx = arg as usize;
                let val = {
                    let f = &self.frames[self.frames.len() - 1];
                    f.fast_locals.get(var_idx).and_then(|v| v.clone())
                };
                match val {
                    // Mirror LOAD_DEREF's own unwrapping: if this slot has
                    // been converted into a cell (MAKE_CELL ran for it, or
                    // a same-slot STORE_FAST landed on an existing cell —
                    // see STORE_FAST's own comment on why that can happen),
                    // push the cell's inner value, not the cell object
                    // itself. Falls through to pushing the raw value
                    // unchanged for the overwhelmingly common non-cell case.
                    Some(v) if matches!(&*v.borrow(), PyObject::Cell { .. }) => {
                        let inner = match &*v.borrow() {
                            PyObject::Cell { value: Some(inner) } => Some(inner.clone()),
                            PyObject::Cell { value: None } => None,
                            _ => unreachable!(),
                        };
                        match inner {
                            Some(inner) => self.frames[fi].push(inner),
                            None => return Err(PyError::unbound_local_error(format!("cannot access local variable '{}' where it is not associated with a value",
                                self.frames[fi].code.varnames.get(var_idx).map_or("?", |&s| crate::interner::lookup_str(s))))),
                        }
                    }
                    Some(v) => self.frames[fi].push(v),
                    None => {
                        if std::env::var("RPY_DEBUG_NAMEERROR").is_ok() {
                            eprintln!(
                                "LOAD_FAST unbound: func={} file={} line={:?} varnames={:?}",
                                self.frames[fi].code.name,
                                self.frames[fi].code.filename,
                                self.frames[fi]
                                    .code
                                    .line_number(self.frames[fi].ip.saturating_sub(1)),
                                self.frames[fi].code.varnames
                            );
                        }
                        return Err(PyError::unbound_local_error(format!("cannot access local variable '{}' where it is not associated with a value",
                            self.frames[fi].code.varnames.get(var_idx).map_or("?", |&s| crate::interner::lookup_str(s)))));
                    }
                }
            }

            Opcode::STORE_FAST => {
                let var_idx = arg as usize;
                let val = self.frames[fi].pop()?;
                let frame = &mut self.frames[fi];
                if var_idx < frame.fast_locals.len() {
                    // If this slot has already been converted into a cell
                    // (MAKE_CELL ran for it, e.g. because a nested
                    // scope/lambda captures the same name — real trigger:
                    // CPython's own `test_listcomps.py`, a comprehension's
                    // iteration variable ending up cellvar-classified),
                    // update the cell's VALUE in place rather than
                    // clobbering the slot with the raw value directly.
                    // Overwriting it outright previously corrupted the
                    // slot's cell-ness, so a LATER `STORE_DEREF`/`LOAD_DEREF`
                    // for the same variable would try to `.borrow_mut()` a
                    // plain (non-Mut) value instead of the expected Cell —
                    // an unconditional hard panic ("borrow_mut called on
                    // non-Mut value"), not just a semantic mismatch.
                    let is_existing_cell = matches!(&frame.fast_locals[var_idx], Some(existing) if matches!(&*existing.borrow(), PyObject::Cell { .. }));
                    if is_existing_cell {
                        if let Some(existing) = frame.fast_locals[var_idx].clone() {
                            if let PyObject::Cell { value } = &mut *existing.borrow_mut() {
                                *value = Some(val.clone());
                            }
                        }
                    } else {
                        frame.fast_locals[var_idx] = Some(val.clone());
                    }
                }
                let name = crate::interner::lookup_str(frame.code.varnames[var_idx]);
                frame.insert_local(name, val);
                if frame.frame_locals_obj.is_some() {
                    self.sync_frame_locals(fi);
                }
            }

            Opcode::LOAD_GLOBAL => {
                let instr_ip = self.frames[fi].ip - 1; // already incremented
                                                       // Check inline cache first — only for globals/module-globals
                                                       // hits. Values resolved from BUILTINS are never cached: the
                                                       // builtins dict can be mutated at runtime
                                                       // (`builtins.len = f`; test_dynamic's test_modify_builtins),
                                                       // and a stale cache would keep returning the old function.
                let mut cached_from_builtins = false;
                if let Some(cached) = self.frames[fi]
                    .global_cache
                    .get(instr_ip)
                    .and_then(|c| c.clone())
                {
                    self.frames[fi].push(cached);
                } else {
                    let name_idx = arg as usize;
                    let name = crate::interner::lookup_str(self.frames[fi].code.names[name_idx]);
                    let mut val = None;
                    {
                        let f = &self.frames[self.frames.len() - 1];
                        if let Some(v) = f.globals.borrow().get(&interner::intern(name)).cloned() {
                            val = Some(v);
                        } else if let Some(v) = f
                            .module_globals
                            .as_ref()
                            .and_then(|mg| mg.borrow().get(&interner::intern(name)).cloned())
                        {
                            val = Some(v);
                        } else {
                            // Builtins: read the LIVE `__builtins__` module
                            // dict so runtime mutations
                            // (`builtins.len = f`; test_dynamic's
                            // test_modify_builtins) are visible — the static
                            // `f.builtins` map is a snapshot never updated.
                            let builtins_mod = f
                                .globals
                                .borrow()
                                .get(&interner::intern("__builtins__"))
                                .cloned()
                                .or_else(|| {
                                    f.module_globals.as_ref().and_then(|mg| {
                                        mg.borrow().get(&interner::intern("__builtins__")).cloned()
                                    })
                                });
                            if let Some(bmod) = builtins_mod {
                                let b = bmod.borrow();
                                if let PyObject::Module { dict, .. } = &*b {
                                    if let Some(v) = dict.get(&interner::intern(name)) {
                                        val = Some(v.clone());
                                    }
                                }
                            }
                            if val.is_none() {
                                if let Some(v) = f.builtins.get(&interner::intern(name)).cloned() {
                                    val = Some(v);
                                    cached_from_builtins = true;
                                }
                            }
                        }
                    }
                    match val {
                        Some(v) => {
                            // Cache for next time (never builtins lookups —
                            // see above).
                            if !cached_from_builtins
                                && instr_ip < self.frames[fi].global_cache.len()
                            {
                                self.frames[fi].global_cache[instr_ip] = Some(v.clone());
                            }
                            self.frames[fi].push(v);
                        }
                        None => return Err(PyError::name_error_for(name)),
                    }
                }
            }

            Opcode::STORE_GLOBAL => {
                let name_idx = arg as usize;
                let name = crate::interner::lookup(self.frames[fi].code.names[name_idx]);
                let val = self.frames[fi].pop()?;
                let sid = interner::intern(&name);
                self.frames[fi].globals.borrow_mut().insert(sid, val.clone());
                // Keep `module.__dict__` in sync with `frame.globals`
                // (see `exec_module_source`'s copying of `module_globals`
                // into `module.dict` — they are separate HashMaps, so a
                // `global` assignment inside a function would otherwise
                // update only `frame.globals` and stay invisible via
                // `module.__dict__` / `module.__dict__['name']` live view,
                // breaking the `__cached_interp_requires_environment` cache
                // in `test.support.script_helper`).
                let mod_name_opt = self.frames[fi]
                    .globals
                    .borrow()
                    .get(&interner::intern("__name__"))
                    .cloned();
                if let Some(mod_name_ref) = mod_name_opt {
                    if let PyObject::Str(s) = &*mod_name_ref.borrow() {
                        if let Some(mod_ref) = self.modules.get(s.as_str()).cloned() {
                            if let PyObject::Module { dict, .. } = &mut *mod_ref.borrow_mut() {
                                dict.insert(sid, val.clone());
                            }
                        }
                    }
                }
                // Also keep `module_globals` in sync for class-body frames
                if let Some(mg) = self.frames[fi].module_globals.clone() {
                    mg.borrow_mut().insert(sid, val);
                }
            }

            Opcode::LOAD_DEREF => {
                let idx = arg as usize;
                let (cell_ref, is_freevar, name_str): (Option<PyObjectRef>, bool, String) = {
                    let f = &self.frames[fi];
                    let code = &f.code;
                    if idx < code.cellvars.len() {
                        let name = &code.cellvars[idx];
                        let var_idx = code
                            .varnames
                            .iter()
                            .position(|&n| crate::interner::intern_eq(n, name))
                            .ok_or_else(|| {
                                PyError::name_error(format!("variable '{}' not found", name))
                            })?;
                        (f.fast_locals[var_idx].clone(), false, name.clone())
                    } else {
                        let fv_idx = idx - code.cellvars.len();
                        let name = code
                            .freevars
                            .get(fv_idx)
                            .ok_or_else(|| PyError::runtime_error("freevar index out of range"))?;
                        (f.closure.get(fv_idx).cloned(), true, name.clone())
                    }
                };
                if let Some(cell) = cell_ref {
                    let val = {
                        let obj = cell.borrow();
                        match &*obj {
                            PyObject::Cell { value: Some(inner) } => inner.clone(),
                            PyObject::Cell { value: None } => {
                                return Err(PyError::name_error_for(&name_str));
                            }
                            _ => cell.clone(),
                        }
                    };
                    self.frames[fi].push(val);
                } else if is_freevar {
                    let val = {
                        let globals = self.frames[fi].globals.borrow();
                        globals.get(&interner::intern(&name_str)).cloned()
                    };
                    if let Some(v) = val {
                        self.frames[fi].push(v);
                    } else {
                        let val = self.frames[fi]
                            .builtins
                            .get(&interner::intern(&name_str))
                            .cloned();
                        if let Some(v) = val {
                            self.frames[fi].push(v);
                        } else {
                            return Err(PyError::name_error_for(&name_str));
                        }
                    }
                } else {
                    return Err(PyError::name_error_for(&name_str));
                }
            }

            Opcode::STORE_DEREF => {
                let idx = arg as usize;
                let val = self.frames[fi].pop()?;
                let has_cellvars = idx < self.frames[fi].code.cellvars.len();
                if has_cellvars {
                    let name = &self.frames[fi].code.cellvars[idx];
                    let var_idx = self.frames[fi]
                        .code
                        .varnames
                        .iter()
                        .position(|&n| crate::interner::intern_eq(n, name))
                        .ok_or_else(|| PyError::runtime_error("variable not found"))?;
                    if var_idx < self.frames[fi].fast_locals.len() {
                        // Defensive: only treat the existing slot value as
                        // a real cell if it actually is one. A same-slot
                        // plain `STORE_FAST` landing here first (see that
                        // opcode's own comment) is now handled there, but
                        // this guards against any OTHER way the slot could
                        // end up non-cell — safe fallback (build a fresh
                        // cell) instead of `.borrow_mut()` unconditionally
                        // panicking on a non-`Mut` value.
                        let existing_is_cell = matches!(&self.frames[fi].fast_locals[var_idx], Some(c) if matches!(&*c.borrow(), PyObject::Cell { .. }));
                        if existing_is_cell {
                            let cell = self.frames[fi].fast_locals[var_idx].clone().unwrap();
                            let mut cell_val = cell.borrow_mut();
                            if let PyObject::Cell { value } = &mut *cell_val {
                                *value = Some(val);
                            }
                        } else {
                            let new_cell = PyObjectRef::new(PyObject::Cell { value: Some(val) });
                            self.frames[fi].fast_locals[var_idx] = Some(new_cell);
                        }
                    } else {
                        let new_cell = PyObjectRef::new(PyObject::Cell { value: Some(val) });
                        self.frames[fi].fast_locals.push(Some(new_cell));
                    }
                } else {
                    let fv_idx = idx - self.frames[fi].code.cellvars.len();
                    let existing_is_cell = matches!(self.frames[fi].closure.get(fv_idx), Some(c) if matches!(&*c.borrow(), PyObject::Cell { .. }));
                    if existing_is_cell {
                        let cell = self.frames[fi].closure[fv_idx].clone();
                        let mut cell_val = cell.borrow_mut();
                        if let PyObject::Cell { value } = &mut *cell_val {
                            *value = Some(val);
                        }
                    } else {
                        return Err(PyError::name_error(format!(
                            "variable '{}' not found",
                            self.frames[fi]
                                .code
                                .freevars
                                .get(fv_idx)
                                .map(|s| s.as_str())
                                .unwrap_or("?")
                        )));
                    }
                }
            }

            Opcode::DELETE_FAST => {
                let var_idx = arg as usize;
                let name = self.frames[fi].code.varnames[var_idx].to_string();
                self.frames[fi].remove_local(&name);
                if self.frames[fi].frame_locals_obj.is_some() {
                    self.sync_frame_locals(fi);
                }
            }

            Opcode::DELETE_NAME => {
                let name_idx = arg as usize;
                let name = self.frames[fi].code.names[name_idx].to_string();
                if let Some(live_module) = self.frames[fi].live_module.clone() {
                    if let PyObject::Module { dict, .. } = &mut *live_module.borrow_mut() {
                        dict.remove(&interner::intern(&name));
                    }
                }
                self.frames[fi]
                    .globals
                    .borrow_mut()
                    .remove(&interner::intern(&name));
                if self.frames[fi].frame_locals_obj.is_some() {
                    self.sync_frame_locals(fi);
                }
            }

            Opcode::POP_TOP => {
                self.frames[fi].pop()?;
            }

            Opcode::DUP_TOP => {
                let val = self.frames[fi].peek(0)?;
                self.frames[fi].push(val);
            }

            Opcode::COPY => {
                let depth = arg as usize;
                if depth >= self.frames[fi].stack.len() {
                    // Graceful fallback: if depth exceeds stack, treat as DUP_TOP
                    if let Some(val) = self.frames[fi].stack.last().cloned() {
                        self.frames[fi].push(val);
                    } else {
                        return Err(PyError::runtime_error("stack underflow (peek)"));
                    }
                } else {
                    let val = self.frames[fi].peek(depth)?;
                    self.frames[fi].push(val);
                }
            }

            Opcode::SWAP => {
                let i = arg as usize;
                let len = self.frames[fi].stack.len();
                if i > 0 && i < len {
                    self.frames[fi].stack.swap(len - 1, len - 1 - i);
                }
            }

            Opcode::RETURN_VALUE => {
                let val = self.frames[fi].pop()?;
                return Ok(Some(val));
            }

            // ── Unimplemented opcode stubs ────────────────────────────
            Opcode::GET_LEN => {
                let obj = self.frames[fi].pop()?;
                let len = crate::object::builtin_len(&[obj])?;
                self.frames[fi].push(len);
            }
            Opcode::MATCH_MAPPING => {
                let subject = self.frames[fi].peek(0)?;
                let is_map = matches!(
                    &*subject.borrow(),
                    PyObject::Dict(_) | PyObject::Instance { .. }
                );
                self.frames[fi].push(py_bool(is_map));
            }
            Opcode::MATCH_SEQUENCE => {
                let subject = self.frames[fi].peek(0)?;
                let is_seq = matches!(
                    &*subject.borrow(),
                    PyObject::List(_)
                        | PyObject::Tuple(_)
                        | PyObject::Str(_)
                        | PyObject::Bytes(_)
                        | PyObject::ByteArray(_)
                );
                self.frames[fi].push(py_bool(is_seq));
            }
            Opcode::MATCH_KEYS => {
                let _keys = self.frames[fi].pop()?;
                // Simplified: always succeed for dict pattern matching
                self.frames[fi].push(py_bool(true));
            }
            Opcode::CALL_INTRINSIC_1 => {
                let intrinsic = arg;
                match intrinsic {
                    1 => {
                        // INTRINSIC_1_INVALIDATION_COUNTER
                        self.frames[fi].push(py_int(0));
                    }
                    2 => {
                        // INTRINSIC_1_PRINT
                        let val = self.frames[fi].pop()?;
                        let _ = print!("{}", val.str());
                        self.frames[fi].push(py_none());
                    }
                    _ => {
                        self.frames[fi].push(py_none());
                    }
                }
            }
            Opcode::CALL_INTRINSIC_2 => {
                // Intrinsics for mutable keys, etc.
                self.frames[fi].push(py_int(0));
            }
            Opcode::UNPACK_SEQUENCE_TWO_TUPLE => {
                let seq = self.frames[fi].pop()?;
                let seq_borrowed = seq.borrow();
                if let PyObject::Tuple(items) = &*seq_borrowed {
                    if items.len() >= 2 {
                        self.frames[fi].push(items[0].clone());
                        self.frames[fi].push(items[1].clone());
                    } else {
                        return Err(PyError::runtime_error("not enough values to unpack"));
                    }
                } else if let PyObject::List(items) = &*seq_borrowed {
                    if items.len() >= 2 {
                        self.frames[fi].push(items[0].clone());
                        self.frames[fi].push(items[1].clone());
                    } else {
                        return Err(PyError::runtime_error("not enough values to unpack"));
                    }
                } else {
                    // Fall back to unpack protocol
                    let it = crate::object::builtin_iter(&[seq.clone()])?;
                    let v1 = crate::object::builtin_next(&[it.clone()])?;
                    let v2 = crate::object::builtin_next(&[it.clone()])?;
                    self.frames[fi].push(v1);
                    self.frames[fi].push(v2);
                }
            }

            // ── Register-based instructions ─────────────────────────
            Opcode::REG_MOV => {
                // Lazily initialize registers
                if self.frames[fi].registers.is_empty() {
                    self.frames[fi].registers = Box::new(vec![None; 256]);
                }
                let dst = (arg >> 4) as usize;
                let src = (arg & 0xF) as usize;
                let val = self.frames[fi].registers[src]
                    .clone()
                    .ok_or_else(|| PyError::runtime_error("REG_MOV: source register is empty"))?;
                if dst < self.frames[fi].registers.len() {
                    self.frames[fi].registers[dst] = Some(val);
                }
            }
            Opcode::REG_LOAD_CONST => {
                let dst = (arg >> 4) as usize;
                let const_idx = (arg & 0xFF) as usize;
                let const_val = self.frames[fi]
                    .code
                    .consts
                    .get(const_idx)
                    .ok_or_else(|| PyError::runtime_error("REG_LOAD_CONST: index out of range"))?
                    .clone();
                let obj = match const_val {
                    ConstValue::None => py_none(),
                    ConstValue::Bool(b) => py_bool(b),
                    ConstValue::Int(s) => {
                        if let Ok(n) = s.parse::<i64>() {
                            py_int(n)
                        } else {
                            let n: BigInt =
                                s.parse().map_err(|_| PyError::value_error("invalid int"))?;
                            PyObjectRef::imm(PyObject::Int(n))
                        }
                    }
                    ConstValue::Float(s) => py_float(
                        s.parse()
                            .map_err(|_| PyError::value_error("invalid float"))?,
                    ),
                    ConstValue::String(s) => py_str(&s),
                    ConstValue::Bytes(b) => PyObjectRef::imm(PyObject::Bytes(b)),
                    ConstValue::Complex { real, imag } => {
                        let re: f64 = real
                            .parse()
                            .map_err(|_| PyError::value_error("invalid complex literal"))?;
                        let im: f64 = imag
                            .parse()
                            .map_err(|_| PyError::value_error("invalid complex literal"))?;
                        PyObjectRef::imm(PyObject::Complex(re, im))
                    }
                    ConstValue::Code(code) => PyObjectRef::imm(PyObject::Code(Rc::from(code))),
                    ConstValue::Tuple(items) => {
                        let objs: Vec<PyObjectRef> =
                            items.into_iter().map(|s| py_str(&s)).collect();
                        PyObjectRef::imm(PyObject::Tuple(objs))
                    }
                };
                if dst < self.frames[fi].registers.len() {
                    self.frames[fi].registers[dst] = Some(obj);
                }
            }
            Opcode::REG_LOAD_FAST => {
                let dst = (arg >> 4) as usize;
                let var_idx = (arg & 0xFF) as usize;
                let val = self.frames[fi].fast_locals.get(var_idx).and_then(|v| v.clone())
                    .ok_or_else(|| PyError::unbound_local_error(format!("cannot access local variable '{}' where it is not associated with a value",
                        self.frames[fi].code.varnames.get(var_idx).map_or("?", |&s| crate::interner::lookup_str(s)))))?;
                if dst < self.frames[fi].registers.len() {
                    self.frames[fi].registers[dst] = Some(val);
                }
            }
            Opcode::REG_STORE_FAST => {
                let src = (arg >> 4) as usize;
                let var_idx = (arg & 0xFF) as usize;
                let val = self.frames[fi].registers[src].clone().ok_or_else(|| {
                    PyError::runtime_error("REG_STORE_FAST: source register is empty")
                })?;
                if var_idx < self.frames[fi].fast_locals.len() {
                    self.frames[fi].fast_locals[var_idx] = Some(val.clone());
                }
                let name = Some(crate::interner::lookup_str(
                    self.frames[fi].code.varnames[var_idx],
                ))
                .ok_or_else(|| PyError::runtime_error("varname index out of range"))?
                .clone();
                self.frames[fi].insert_local(&name, val);
            }
            Opcode::REG_BINARY_OP => {
                let dst = (arg >> 4) as usize;
                let a_reg = ((arg >> 2) & 0x3) as usize;
                let b_reg = (arg & 0x3) as usize;
                let op = (arg >> 8) as u32;
                let a = self.frames[fi].registers[a_reg]
                    .clone()
                    .ok_or_else(|| PyError::runtime_error("REG_BINARY_OP: a is empty"))?;
                let b = self.frames[fi].registers[b_reg]
                    .clone()
                    .ok_or_else(|| PyError::runtime_error("REG_BINARY_OP: b is empty"))?;
                let result = match op {
                    0 => py_add(&a, &b),
                    1 => py_sub(&a, &b),
                    2 => py_mul(&a, &b),
                    3 => py_div(&a, &b),
                    4 => py_floor_div(&a, &b),
                    5 => py_mod(&a, &b),
                    6 => py_pow(&a, &b),
                    7 => py_lshift(&a, &b),
                    8 => py_rshift(&a, &b),
                    9 => py_bit_or(&a, &b),
                    10 => py_bit_xor(&a, &b),
                    11 => py_bit_and(&a, &b),
                    13 => py_getitem(&a, &b),
                    _ => {
                        return Err(PyError::runtime_error(format!(
                            "unknown reg binary op: {}",
                            op
                        )))
                    }
                }?;
                if dst < self.frames[fi].registers.len() {
                    self.frames[fi].registers[dst] = Some(result);
                }
            }
            Opcode::REG_LOAD_GLOBAL => {
                let dst = (arg >> 4) as usize;
                let name_idx = (arg & 0xFF) as usize;
                let name = crate::interner::lookup_str(self.frames[fi].code.names[name_idx]);
                // Check inline cache first
                let instr_ip = self.frames[fi].ip - 1;
                if let Some(cached) = self.frames[fi]
                    .global_cache
                    .get(instr_ip)
                    .and_then(|c| c.clone())
                {
                    if dst < self.frames[fi].registers.len() {
                        self.frames[fi].registers[dst] = Some(cached);
                    }
                } else {
                    let val = self.frames[fi]
                        .globals
                        .borrow()
                        .get(&interner::intern(name))
                        .cloned()
                        .or_else(|| {
                            self.frames[fi]
                                .builtins
                                .get(&interner::intern(name))
                                .cloned()
                        });
                    if let Some(v) = val {
                        if instr_ip < self.frames[fi].global_cache.len() {
                            self.frames[fi].global_cache[instr_ip] = Some(v.clone());
                        }
                        if dst < self.frames[fi].registers.len() {
                            self.frames[fi].registers[dst] = Some(v);
                        }
                    } else {
                        return Err(PyError::name_error(format!(
                            "name '{}' is not defined",
                            name
                        )));
                    }
                }
            }
            Opcode::REG_RETURN => {
                let src = (arg & 0xFF) as usize;
                let val = self.frames[fi].registers[src]
                    .clone()
                    .ok_or_else(|| PyError::runtime_error("REG_RETURN: register is empty"))?;
                return Ok(Some(val));
            }
            Opcode::REG_BUILD_LIST => {
                // arg: upper 4 bits = dst, lower 4 bits = count
                let dst = (arg >> 4) as usize;
                let count = (arg & 0xF) as usize;
                let mut items = Vec::with_capacity(count);
                for i in 0..count {
                    if let Some(val) = self.frames[fi].registers[i].clone() {
                        items.push(val);
                    }
                }
                if dst < self.frames[fi].registers.len() {
                    self.frames[fi].registers[dst] = Some(py_list(items));
                }
            }

            Opcode::PUSH_NULL => {
                self.frames[fi].push(py_none());
            }

            Opcode::CALL => {
                let npos = arg as usize & 0xFF;
                let nkw = (arg as usize >> 8) & 0xFF;
                let stack_len = self.frames[fi].stack.len();

                // FAST PATH: plain positional call -- stack is exactly
                // [callable, a1..aN]. split_off moves everything out in one
                // allocation; no per-item clone, no reverse, no intermediate
                // items Vec. `tail.remove(0)` shifts at most a handful of
                // elements for typical arities.
                if nkw == 0 && npos + 1 <= stack_len {
                    let split = stack_len - 1 - npos;
                    let callable = self.frames[fi].stack.remove(split);
                    let args: Vec<PyObjectRef> =
                        self.frames[fi].stack.drain(split..).collect();

                    // METHOD FAST PATH: bound native method with positional
                    // args only -- invoke the fn pointer directly instead of
                    // re-entering call_function (which would rebuild yet
                    // another Vec and re-dispatch).
                    {
                        let is_special_throw = matches!(
                            &*callable.borrow(),
                            PyObject::BuiltinMethod { func, .. }
                                if std::ptr::fn_addr_eq(
                                    *func,
                                    crate::object::generator_throw_fallback as crate::object::BuiltinFunc,
                                )
                        );
                        if !is_special_throw {
                            if let PyObject::BuiltinMethod { name, func, self_obj } =
                                &*callable.borrow()
                            {
                                // Ultra-hot primitive: list.append(x) -> None
                                if name == "append" && args.len() == 1 {
                                    let so = self_obj.clone();
                                    let mut b = so.borrow_mut();
                                    if let PyObject::List(items) = &mut *b {
                                        items.push(args[0].clone());
                                        drop(b);
                                        self.frames[fi].push(py_none());
                                        return Ok(None);
                                    }
                                }
                                let mut na = Vec::with_capacity(args.len() + 1);
                                na.push(self_obj.clone());
                                na.extend(args.iter().cloned());
                                drop(callable.borrow());
                                let r = func(&na)?;
                                self.frames[fi].push(r);
                                return Ok(None);
                            }
                        }
                    }

                    let result = self.call_function(callable, args, vec![])?;
                    self.frames[fi].push(result);
                    return Ok(None);
                }

                // General path: kwargs and/or **kwargs present. Stack shape:
                // [callable, args..., kw_name, kw_val, ..., **kwargs_dict]
                let total_to_pop = npos + 2 * nkw;
                let mut items = Vec::with_capacity(total_to_pop);
                for _ in 0..total_to_pop {
                    if self.frames[fi].stack.len() > 1 {
                        items.push(self.frames[fi].pop()?);
                    } else {
                        break;
                    }
                }
                let callable = self.frames[fi].pop()?;
                items.reverse();
                // Separate positional args and keywords
                let mut args = Vec::new();
                let mut keywords = Vec::new();
                let mut i = 0;
                while i < npos && i < items.len() {
                    args.push(items[i].clone());
                    i += 1;
                }
                while i + 1 < items.len() {
                    if let PyObject::Str(name) = &*items[i].borrow() {
                        keywords.push((name.to_string(), items[i + 1].clone()));
                        i += 2;
                    } else {
                        break;
                    }
                }
                let result = self.call_function(callable, args, keywords)?;
                self.frames[fi].push(result);
            }
            Opcode::MAKE_CELL => {
                let idx = arg as usize;
                let frame = &mut self.frames[fi];
                if idx < frame.fast_locals.len() {
                    let val = frame.fast_locals[idx].take();
                    let cell = PyObjectRef::new(PyObject::Cell { value: val });
                    frame.fast_locals[idx] = Some(cell);
                }
            }

            Opcode::COPY_FREE_VARS => {
                let nfree = arg as usize;
                let mut cells = Vec::with_capacity(nfree);
                for _ in 0..nfree {
                    cells.push(self.frames[fi].pop()?);
                }
                // Store the closure tuple on the stack for MAKE_FUNCTION to consume
                self.frames[fi].push(PyObjectRef::imm(PyObject::Tuple(cells)));
            }

            Opcode::MAKE_FUNCTION => {
                let has_closure = (arg & 0x100) != 0;
                let n_defaults = (arg & 0xFF) as usize;
                let n_kwdefaults = ((arg >> 9) & 0xFF) as usize;
                // Stack (bottom to top): [closure?, CODE, pos_defaults...,
                // kwonly_defaults...] — kwonly defaults were pushed last, so
                // pop them first. Appended after positional defaults in the
                // final `defaults` vec (see CodeObject::kwonly_defaults_mask
                // for how call-binding tells the two apart).
                let mut kwdefaults = Vec::new();
                for _ in 0..n_kwdefaults {
                    kwdefaults.push(self.frames[fi].pop()?);
                }
                kwdefaults.reverse();
                let mut defaults = Vec::new();
                for _ in 0..n_defaults {
                    defaults.push(self.frames[fi].pop()?);
                }
                defaults.reverse();
                defaults.extend(kwdefaults);
                let code_obj = self.frames[fi].pop()?;
                // A cheap `Rc` clone, not a deep copy of the whole
                // `CodeObject` (instructions, consts, ...) — this used to
                // `.clone()` the dereferenced `CodeObject` itself here,
                // meaning a `def`/`lambda` executed fresh on every
                // iteration of a loop deep-cloned its entire compiled body
                // every single time, even though (with `LOAD_CONST`'s own
                // caching) the SAME `PyObject::Code` constant was being
                // read repeatedly.
                let code = match &*code_obj.borrow() {
                    PyObject::Code(c) => c.clone(),
                    _ => {
                        return Err(PyError::runtime_error(
                            "MAKE_FUNCTION: expected code object",
                        ))
                    }
                };
                let closure = if has_closure {
                    let closure_tuple = self.frames[fi].pop()?;
                    let items = closure_tuple.borrow();
                    if let PyObject::Tuple(items) = &*items {
                        items.clone()
                    } else {
                        Vec::new()
                    }
                } else {
                    Vec::new()
                };
                // Use module_globals when available (class body execution) so that
                // functions defined inside a class body capture the module's globals
                // (e.g. 'empty' from django.utils.functional) rather than the class
                // namespace. Falls back to the frame's globals for module-level code
                // and regular function calls.
                let globals = self.frames[fi]
                    .module_globals
                    .clone()
                    .unwrap_or_else(|| self.frames[fi].globals.clone());
                let code_obj = code.clone();
                let func = PyObjectRef::new(PyObject::Function(Box::new(PyFunction {
                    code: code_obj.clone(),
                    globals,
                    defaults,
                    closure,
                    dict: HashMap::new(),
                    jit_ptr: std::cell::Cell::new(0),
                    jit_consts: std::cell::RefCell::new(Vec::new()),
                })));
                // Set __code__ and __module__ on the function
                if let PyObject::Function(ref mut inner_f) = &mut *func.borrow_mut() {
                    let dict = &mut inner_f.dict;
                    dict.insert_str("__code__", PyObjectRef::imm(PyObject::Code(code_obj)));
                }
                let module_name_opt: Option<String> = if let Some(ref mg) = self.frames[fi].module_globals {
                    let mg = mg.borrow();
                    mg.get(&interner::intern("__name__")).and_then(|v| {
                        if let PyObject::Str(s) = &*v.borrow() { Some(s.to_string()) } else { None }
                    })
                } else {
                    let g = self.frames[fi].globals.borrow();
                    g.get(&interner::intern("__name__")).and_then(|v| {
                        if let PyObject::Str(s) = &*v.borrow() { Some(s.to_string()) } else { None }
                    })
                };
                if let Some(s) = module_name_opt {
                    if let PyObject::Function(ref mut inner_f) = &mut *func.borrow_mut() {
                        let dict = &mut inner_f.dict;
                        dict.insert_str("__module__", py_str(&s));
                    }
                }
                self.frames[fi].push(func);
            }

            Opcode::BUILD_LIST
            | Opcode::BUILD_TUPLE
            | Opcode::BUILD_MAP
            | Opcode::BUILD_SET
            | Opcode::BUILD_STRING
            | Opcode::BUILD_SLICE
            | Opcode::BINARY_OP
            | Opcode::SUPER_FAST2_BIN
            | Opcode::SUPER_FASTC_BIN
            | Opcode::SUPER_FAST_MOV
            | Opcode::COMPARE_OP
            | Opcode::IS_OP
            | Opcode::CONTAINS_OP
            | Opcode::UNARY_NEGATIVE
            | Opcode::UNARY_POSITIVE
            | Opcode::UNARY_NOT
            | Opcode::UNARY_INVERT
            | Opcode::JUMP_FORWARD
            | Opcode::JUMP
            | Opcode::JUMP_BACKWARD
            | Opcode::POP_JUMP_IF_FALSE
            | Opcode::POP_JUMP_IF_TRUE
            | Opcode::POP_JUMP_IF_NONE
            | Opcode::POP_JUMP_IF_NOT_NONE => {
                let _ = self.handle_build_arith_control(fi, op, arg)?;
            }

            Opcode::GET_ITER => {
                let val = self.frames[fi].pop()?;
                // Check for user-class instance (needs __iter__ protocol)
                let is_instance = val.borrow().type_name() == "instance";
                if is_instance {
                    // A class transparently subclassing list/dict/str
                    // (`class Foo(list): ...`) with no __iter__ override
                    // should iterate its real native backing directly —
                    // list/dict don't define "__iter__" as a plain
                    // get_attribute entry (iteration normally goes through
                    // this same opcode's native match instead), so routing
                    // it through get_attribute below would silently miss and
                    // fall into the unrelated dict-like-instance fallback.
                    let has_override = if let PyObject::Instance { typ, .. } = &*val.borrow() {
                        crate::object::lookup_dunder_via_mro(typ, "__iter__").is_some()
                    } else {
                        false
                    };
                    if !has_override {
                        if let Some(native) = crate::object::native_backing_of(&val) {
                            let iterator = crate::object::builtin_iter(&[native])?;
                            self.frames[fi].push(iterator);
                            return Ok(None);
                        }
                        // No `__iter__` override (confirmed via the mro
                        // lookup above) and no native backing — delegate
                        // to `builtin_iter`, which implements the real
                        // "no `__iter__`, fall back to `__getitem__`"
                        // protocol (`for x in obj:` calling `obj[0]`,
                        // `obj[1]`, ... until `IndexError`) and raises a
                        // clean `TypeError` if neither exists. Previously
                        // this fell through to `get_attribute("__iter__")`
                        // below even with `has_override` already known
                        // false, which doesn't raise cleanly for a plain
                        // instance with no `__iter__` — real trigger:
                        // `for x in SequenceClass(3): ...` (an object with
                        // only `__getitem__`, the standard old-style
                        // sequence-iteration idiom) silently misbehaved
                        // instead of iterating 0, 1, 2.
                        let iterator = crate::object::builtin_iter(&[val.clone()])?;
                        self.frames[fi].push(iterator);
                        return Ok(None);
                    }
                    use crate::object::ObjectAccess;
                    let raw_method = val.borrow().get_attribute("__iter__").map_err(|_| {
                        PyError::type_error(format!(
                            "'{}' object is not iterable",
                            val.borrow().type_name()
                        ))
                    })?;
                    let val_clone = val.clone();
                    let iter_method = PyObjectRef::imm(PyObject::BoundMethod {
                        func: raw_method,
                        self_obj: val_clone,
                    });
                    // LAZY iteration (CPython semantics): `__iter__` returns
                    // THE iterator; FOR_ITER advances it one step at a time.
                    let iterator = self.call_function(iter_method, vec![], vec![])?;
                    self.frames[fi].push(iterator);
                } else {
                    let obj = val.borrow();
                    match &*obj {
                        PyObject::List(v) => {
                            self.frames[fi].push(PyObjectRef::new(PyObject::ListIter {
                                list: v.clone(),
                                index: 0,
                            }));
                        }
                        PyObject::Deque { data, .. } => {
                            self.frames[fi].push(PyObjectRef::new(PyObject::DequeIter {
                                deque: val.clone(),
                                index: 0,
                                start_len: data.len(),
                            }));
                        }
                        PyObject::Tuple(v) => {
                            self.frames[fi].push(PyObjectRef::new(PyObject::ListIter {
                                list: v.clone(),
                                index: 0,
                            }));
                        }
                        PyObject::Str(s) => {
                            let chars: Vec<PyObjectRef> =
                                s.chars().map(|c| py_str(&c.to_string())).collect();
                            self.frames[fi].push(PyObjectRef::new(PyObject::ListIter {
                                list: chars,
                                index: 0,
                            }));
                        }
                        // `FrozenSet` was missing from this match entirely
                        // (only mutable `Set` was handled) — `for x in
                        // frozenset(...):`/`for x in some_frozenset:` fell to
                        // the `_` catch-all below and raised `TypeError:
                        // 'frozenset' object is not iterable` outright, a
                        // foundational gap for one of Python's basic builtin
                        // container types. `builtin_iter`'s OWN, separate
                        // FrozenSet handling (used by `iter()`/`list()`/etc.,
                        // not by a `for` STATEMENT, which compiles to this
                        // opcode instead) had the identical gap, fixed
                        // alongside this one.
                        PyObject::Set(s) | PyObject::FrozenSet(s) => {
                            self.frames[fi].push(PyObjectRef::new(PyObject::ListIter {
                                list: s.to_vec(),
                                index: 0,
                            }));
                        }
                        PyObject::Bytes(b) => {
                            let items: Vec<PyObjectRef> =
                                b.iter().map(|byte| py_int(*byte as i64)).collect();
                            self.frames[fi].push(PyObjectRef::new(PyObject::ListIter {
                                list: items,
                                index: 0,
                            }));
                        }
                        PyObject::ByteArray(b) => {
                            let items: Vec<PyObjectRef> =
                                b.iter().map(|byte| py_int(*byte as i64)).collect();
                            self.frames[fi].push(PyObjectRef::new(PyObject::ListIter {
                                list: items,
                                index: 0,
                            }));
                        }
                        PyObject::Array(arr) => {
                            let items: Vec<PyObjectRef> = arr
                                .data
                                .iter()
                                .map(|v| {
                                    if crate::object::array_typecode_is_float(arr.typecode) {
                                        py_float(*v)
                                    } else if arr.typecode == 'w' || arr.typecode == 'u' {
                                        let ch = (*v as u32).try_into().ok().and_then(char::from_u32).unwrap_or('\0');
                                        py_str(&ch.to_string())
                                    } else {
                                        py_int(*v as i64)
                                    }
                                })
                                .collect();
                            self.frames[fi].push(PyObjectRef::new(PyObject::ListIter {
                                list: items,
                                index: 0,
                            }));
                        }
                        PyObject::MemoryView { .. } => {
                            drop(obj);
                            let iterator = crate::object::builtin_iter(&[val.clone()])?;
                            self.frames[fi].push(iterator);
                        }
                        PyObject::Generator { .. } => {
                            drop(obj);
                            self.frames[fi].push(val);
                        }
                        PyObject::Range { start, stop, step } => {
                            self.frames[fi].push(PyObjectRef::new(PyObject::RangeIter {
                                current: start.clone(),
                                stop: stop.clone(),
                                step: step.clone(),
                            }));
                        }
                        PyObject::Dict(ref pydict) => {
                            let keys: Vec<PyObjectRef> = pydict.keys();
                            self.frames[fi].push(PyObjectRef::new(PyObject::ListIter {
                                list: keys,
                                index: 0,
                            }));
                        }
                        PyObject::Globals(g) => {
                            let keys: Vec<PyObjectRef> = g
                                .borrow()
                                .keys()
                                .map(|k| py_str(crate::interner::lookup_str(*k)))
                                .collect();
                            self.frames[fi].push(PyObjectRef::new(PyObject::ListIter {
                                list: keys,
                                index: 0,
                            }));
                        }
                        PyObject::EnumerateIter { .. } => {
                            drop(obj);
                            self.frames[fi].push(val);
                        }
                        // Iterators are their own iterator (matching CPython's
                        // `__iter__` returning self) — `for x in iter(y):` or
                        // `for x in itertools.tee(y)[0]:` must work the same as
                        // iterating the original iterable directly.
                        PyObject::ListIter { .. }
                        | PyObject::RangeIter { .. }
                        | PyObject::MapIterator { .. }
                        | PyObject::FilterIterator { .. }
                        | PyObject::ZipIterator { .. }
                        | PyObject::CycleIter { .. }
                        | PyObject::GroupByIter { .. }
                        | PyObject::GetItemIter { .. }
                        | PyObject::CallSentinelIter { .. } => {
                            drop(obj);
                            self.frames[fi].push(val);
                        }
                        // A class object itself can be iterable via its
                        // metaclass's `__iter__` (e.g. `for member in
                        // SomeEnum:` — `SomeEnum` is a `PyObject::Type`, and
                        // `__iter__` lives on its metaclass, not on `SomeEnum`
                        // own dict/mro, which is why this needs metatype_of
                        // rather than the ordinary Type attribute lookup above).
                        PyObject::Type { .. } => {
                            let iter_fn = crate::object::metatype_of(&val).and_then(|mt| {
                                if let PyObject::Type { mro, .. } = &*mt.borrow() {
                                    for base in mro.iter() {
                                        if let PyObject::Type { dict, .. } = &*base.borrow() {
                                            if let Some(v) = dict.get_str("__iter__") {
                                                return Some(v.clone());
                                            }
                                        }
                                    }
                                }
                                None
                            });
                            drop(obj);
                            match iter_fn {
                                Some(f) => {
                                    let iterator =
                                        self.call_function(f, vec![val.clone()], vec![])?;
                                    self.frames[fi].push(iterator);
                                }
                                None => {
                                    return Err(PyError::type_error(format!(
                                        "'{}' object is not iterable",
                                        val.get_type_name()
                                    )))
                                }
                            }
                        }
                        // `for line in open(path):` — one of the single most
                        // common real-Python file-reading idioms — was entirely
                        // unhandled (`TypeError: 'file' object is not
                        // iterable`), confirmed via `Lib/dbm/dumb.py`'s own
                        // `_update` (`for line in f:` over its index file), but
                        // the gap is completely general, not dbm-specific.
                        // Reads the whole remaining content and splits it into
                        // lines (keeping each line's own trailing `\n`, matching
                        // real `readline()`/iteration semantics) — eager,
                        // matching every other native-type arm in this same
                        // match (`List`/`Tuple`/`Str`/...), not the lazy
                        // `CallSentinelIter` `readline()`-driven approach used
                        // by this project's OWN `readline`/`__next__` methods
                        // (added alongside this fix, `attrs.rs`) for direct
                        // `f.readline()`/`next(f)` calls.
                        PyObject::File { file, binary, .. } => {
                            use std::io::Read;
                            let binary = *binary;
                            let mut rest = Vec::new();
                            file.borrow_mut()
                                .read_to_end(&mut rest)
                                .map_err(|e| PyError::os_error_from_io(&e))?;
                            drop(obj);
                            let mut lines: Vec<PyObjectRef> = Vec::new();
                            let mut current: Vec<u8> = Vec::new();
                            for byte in rest {
                                current.push(byte);
                                if byte == b'\n' {
                                    lines.push(if binary {
                                        PyObjectRef::imm(PyObject::Bytes(current.clone()))
                                    } else {
                                        py_str(&String::from_utf8_lossy(&current))
                                    });
                                    current.clear();
                                }
                            }
                            if !current.is_empty() {
                                lines.push(if binary {
                                    PyObjectRef::imm(PyObject::Bytes(current.clone()))
                                } else {
                                    py_str(&String::from_utf8_lossy(&current))
                                });
                            }
                            self.frames[fi].push(PyObjectRef::new(PyObject::ListIter {
                                list: lines,
                                index: 0,
                            }));
                        }
                        _ => {
                            let type_name = obj.type_name();
                            drop(obj);
                            match crate::object::builtin_iter(&[val.clone()]) {
                                Ok(it) => self.frames[fi].push(it),
                                Err(_) => {
                                    return Err(PyError::type_error(format!(
                                        "'{}' object is not iterable",
                                        type_name
                                    )))
                                }
                            }
                        }
                    }
                }
            }

            Opcode::FOR_ITER => {
                let iter_val = self.frames[fi].peek(0)?;
                let is_generator = matches!(&*iter_val.borrow(), PyObject::Generator { .. });
                if is_generator {
                    // Call __next__ on generator
                    let gen = iter_val.clone();
                    let next_func = gen.borrow().get_attribute("__next__");
                    if let Ok(next_func) = next_func {
                        // Fix self_obj by extracting name and func
                        let (n, f) = {
                            let b = next_func.borrow();
                            if let PyObject::BuiltinMethod { name, func, .. } = &*b {
                                (name.clone(), *func)
                            } else {
                                return Err(PyError::runtime_error("expected __next__ method"));
                            }
                        };
                        let fixed = PyObjectRef::imm(PyObject::BuiltinMethod {
                            name: n,
                            func: f,
                            self_obj: gen.clone(),
                        });
                        match self.call_function(fixed, vec![], vec![]) {
                            Ok(val) => {
                                self.frames[fi].push(val);
                            }
                            // A generator's __next__/send driver signals
                            // normal exhaustion via an ad hoc
                            // `PyError::Exception("StopIteration", return_value)`
                            // (see its get_attribute arm), not the plain
                            // `PyError::StopIteration` variant — checking
                            // only the latter here meant `for x in
                            // some_generator(): ...` never terminated
                            // cleanly and instead leaked as an uncaught
                            // exception once the generator was exhausted.
                            Err(e) if crate::object::is_stop_iteration_error(&e) => {
                                self.frames[fi].ip = arg as usize;
                            }
                            Err(e) => return Err(e),
                        }
                    } else {
                        self.frames[fi].ip = arg as usize;
                    }
                } else {
                    let is_exhausted = {
                        let obj = iter_val.borrow();
                        match &*obj {
                            PyObject::List(v) => v.is_empty(),
                            PyObject::ListIter { list, index } => *index >= list.len(),
                            PyObject::RangeIter {
                                current,
                                stop,
                                step,
                            } => {
                                if step.sign() == num_bigint::Sign::Plus {
                                    *current >= *stop
                                } else {
                                    *current <= *stop
                                }
                            }
                            // ZipIterator/MapIterator/FilterIterator don't fit
                            // this branch's exhausted-check-then-advance shape
                            // (advancing several sub-iterators, e.g. zip's, in
                            // lockstep isn't a simple index/length compare) —
                            // `builtin_next` already implements all of that
                            // correctly (it's what list()/sum()/etc. already go
                            // through), so drop straight into it here instead
                            // of duplicating that logic. Previously these three
                            // fell to the `_` arm below and raised "for_iter on
                            // non-iterable" — i.e. `for x in zip(a, b):` (or
                            // map/filter) used directly as a for-loop target,
                            // as opposed to being wrapped in `list(...)` first,
                            // has never worked.
                            // `CycleIter` (`itertools.cycle`) shares the same
                            // "doesn't fit index/length exhaustion" shape —
                            // genuinely infinite (wraps via modulo), so there's
                            // no `len()` to compare against at all; delegate to
                            // `builtin_next` exactly like Zip/Map/Filter above.
                            // `EnumerateIter` moved here too — it no longer
                            // holds a materialized `items`/`len()` to compare
                            // against now that it's a lazy wrapper around a
                            // `source` iterator (see its own doc comment).
                            PyObject::ZipIterator { .. }
                            | PyObject::MapIterator { .. }
                            | PyObject::FilterIterator { .. }
                            | PyObject::CycleIter { .. }
                            | PyObject::EnumerateIter { .. }
                            | PyObject::GroupByIter { .. }
                            | PyObject::GetItemIter { .. }
                            | PyObject::CallSentinelIter { .. }
                            | PyObject::DequeIter { .. }
                            | PyObject::DequeRevIter { .. } => {
                                drop(obj);
                                match crate::object::builtin_next(&[iter_val.clone()]) {
                                    Ok(val) => {
                                        self.frames[fi].push(val);
                                    }
                                    Err(e) if crate::object::is_stop_iteration_error(&e) => {
                                        self.frames[fi].ip = arg as usize;
                                    }
                                    Err(e) => return Err(e),
                                }
                                return Ok(None);
                            }
                            _ => {
                                // Not a built-in iterator — check for __next__ protocol.
                                // MUST drop(obj) before delegating: `obj` is a live
                                // borrow of the iterator, and __next__ legitimately
                                // mutates it (self.i += 1 in user code) -> borrow_mut
                                // on the same object while this borrow is held
                                // panicked with "RefCell already borrowed" for ANY
                                // `yield from custom_iter` / `for x in custom:`
                                // whose __next__ stores attributes.
                                let is_inst = obj.type_name() == "instance";
                                drop(obj);
                                if is_inst {
                                    return self.for_iter_next(iter_val.clone(), arg);
                                }
                                return Err(PyError::type_error("for_iter on non-iterable"));
                            }
                        }
                    };
                    if is_exhausted {
                        self.frames[fi].ip = arg as usize;
                    } else {
                        let val = self.frames[fi].pop()?;
                        let item = {
                            // Convert plain List to ListIter for O(1) iteration
                            let is_plain_list = matches!(&*val.borrow(), PyObject::List(..));
                            if is_plain_list {
                                let list_clone = {
                                    let obj = val.borrow();
                                    if let PyObject::List(v) = &*obj {
                                        v.clone()
                                    } else {
                                        unreachable!()
                                    }
                                };
                                *val.borrow_mut() = PyObject::ListIter {
                                    list: list_clone,
                                    index: 0,
                                };
                            }
                            let mut obj = val.borrow_mut();
                            match &mut *obj {
                                PyObject::ListIter { list, index } => {
                                    let v = list[*index].clone();
                                    *index += 1;
                                    v
                                }
                                PyObject::RangeIter {
                                    current,
                                    stop: _,
                                    step,
                                } => {
                                    let v = py_int(current.clone());
                                    *current += &*step;
                                    v
                                }
                                // `EnumerateIter` no longer reaches this arm at
                                // all — it moved to the earlier "delegate to
                                // builtin_next, return early" bucket above
                                // (alongside Zip/Map/Filter/Cycle) once it
                                // became a lazy `source`-wrapper instead of a
                                // materialized `items` list with no `len()` to
                                // compare against.
                                _ => unreachable!(),
                            }
                        };
                        self.frames[fi].push(val);
                        self.frames[fi].push(item);
                    }
                }
            }

            Opcode::LOAD_ATTR => {
                let name_idx = arg as usize;
                let name = crate::interner::lookup(self.frames[fi].code.names[name_idx]);
                let obj = self.frames[fi].pop()?;
                let obj = deref_proxy(&obj)?;
                let result = {
                    let obj_borrowed = obj.borrow();
                    match &*obj_borrowed {
                        // `it.__next__()`/`it.__iter__()` on any of this
                        // codebase's iterator shapes: `attrs.rs`'s
                        // `get_attribute_impl` (called from `&PyObject`,
                        // with no access to the enclosing `PyObjectRef`)
                        // can only bind a FRESH CLONE of the iterator as
                        // `self_obj` — for a stateful iterator (advancing
                        // via a mutable `index` field), that silently
                        // disconnects the returned method from the real
                        // object: every call to `it.__next__()` re-read the
                        // SAME starting state instead of advancing
                        // (confirmed via direct repro: three successive
                        // `it.__next__()` calls all returned the first
                        // element). Handled here instead, where the real
                        // `obj` `PyObjectRef` is available to bind directly
                        // — same fix shape as `GET_AWAITABLE`'s own
                        // `self_obj`-rebind elsewhere in this file, for the
                        // identical underlying limitation.
                        PyObject::ListIter { .. }
                        | PyObject::RangeIter { .. }
                        | PyObject::MapIterator { .. }
                        | PyObject::FilterIterator { .. }
                        | PyObject::ZipIterator { .. }
                        | PyObject::CycleIter { .. }
                        | PyObject::GroupByIter { .. }
                        | PyObject::EnumerateIter { .. }
                        | PyObject::GetItemIter { .. }
                        | PyObject::CallSentinelIter { .. }
                            if name == "__next__" || name == "__iter__" =>
                        {
                            let func: crate::object::BuiltinFunc = if name == "__next__" {
                                crate::object::builtin_next
                            } else {
                                crate::object::builtin_iter
                            };
                            Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                                name: name.clone(),
                                func,
                                self_obj: obj.clone(),
                            }))
                        }
                        // `range_iterator`/`list_iterator.__setstate__(state)`
                        // (real CPython's pickle-restore protocol, also
                        // directly usable) — needs the same real-`self_obj`
                        // treatment as `__next__`/`__iter__` just above,
                        // since it MUTATES the iterator's position in place
                        // (a disconnected clone would silently do nothing).
                        // Found via CPython's own `test_range.py::
                        // test_iterator_setstate`.
                        PyObject::RangeIter { .. } if name == "__setstate__" => {
                            Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                                name: name.clone(),
                                func: crate::object::range_iter_setstate,
                                self_obj: obj.clone(),
                            }))
                        }
                        PyObject::ListIter { .. } if name == "__setstate__" => {
                            Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                                name: name.clone(),
                                func: crate::object::list_iter_setstate,
                                self_obj: obj.clone(),
                            }))
                        }
                        PyObject::Super {
                            cls: _,
                            obj: _super_obj,
                        } => {
                            // super(cls, obj).attr: walk MRO of obj's type, starting after cls
                            drop(obj_borrowed);
                            let attr = obj.borrow().get_attribute(&name)?;
                            Ok(attr)
                        }
                        PyObject::Function(_) if name == "__dict__" => {
                            // `func.__dict__` as a LIVE view backed by the
                            // function's real dict (writes through via
                            // `PyDict::set_with_hash`'s Function arm) — the
                            // get_attribute_impl snapshot copy made the common
                            // `func.__dict__['x'] = v` decorator-helper
                            // pattern silently write into a discarded copy.
                            // Mirrors the Instance `__dict__` live-view right
                            // below.
                            let mut pd = crate::object::PyDict::new();
                            if let PyObject::Function(f) = &*obj.borrow() {
                                for (k, v) in f.dict.iter() {
                                    if k.starts_with("__") && k.ends_with("__") {
                                        continue;
                                    }
                                    let _ = pd.set(py_str(k), v.clone());
                                }
                            }
                            drop(obj_borrowed);
                            pd.instance_ref = Some(obj.clone());
                            self.frames[fi].push(PyObjectRef::new(PyObject::Dict(Box::new(pd))));
                            return Ok(None);
                        }
                        PyObject::Module { .. } if name == "__dict__" => {
                            let mut pd = crate::object::PyDict::new();
                            if let PyObject::Module { dict, .. } = &*obj.borrow() {
                                for (k, v) in dict.iter() {
                                    let key = py_str(crate::interner::lookup_str(*k));
                                    let _ = pd.set(key, v.clone());
                                }
                            }
                            drop(obj_borrowed);
                            pd.instance_ref = Some(obj.clone());
                            self.frames[fi].push(PyObjectRef::new(PyObject::Dict(Box::new(pd))));
                            return Ok(None);
                        }
                        PyObject::Instance { dict, typ } => {
                            // Inline attribute cache: skip full lookup if cached
                            // with matching type tag — only valid when this
                            // instance's OWN dict doesn't also define `name`,
                            // since the cache only ever stores type/mro-level
                            // hits (methods, class attributes); an instance
                            // that shadows the class attribute with its own
                            // instance-level value of the same name must still
                            // win over a stale cache entry from some OTHER
                            // instance of the same type that didn't have that
                            // override (see the caching-site comment below for
                            // the matching write-side half of this fix).
                            let type_tag = typ.get_id() as u64;
                            let cached = if dict.contains_key(&name) {
                                None
                            } else {
                                self.frames[fi]
                                    .attr_cache
                                    .get(name_idx)
                                    .and_then(|entry| entry.as_ref())
                                    .filter(|(tag, _)| *tag == type_tag)
                                    .map(|(_, val)| val.clone())
                            };
                            if let Some(cached_val) = cached {
                                // The cached value may be a method already
                                // BOUND to whatever instance first populated
                                // this cache slot (`self_obj` baked in) — the
                                // cache itself is keyed only by
                                // `(name_idx, type_tag)`, with no per-
                                // instance component, so reusing it AS-IS
                                // for a DIFFERENT instance of the same type
                                // silently operated on the wrong `self`
                                // (confirmed via a direct repro: calling the
                                // same bound-method-shaped attribute on two
                                // different instances of the same class
                                // within one frame — e.g.
                                // `subprocess.CompletedProcess.check_returncode`
                                // — the second call silently used the
                                // FIRST instance as `self`). Rebind to the
                                // CURRENT `obj` before returning, matching
                                // the same rebind-on-hit fix already applied
                                // to the OTHER (module-level) attribute
                                // cache just above in this file.
                                //
                                // Only rebind when the cached `self_obj` is
                                // ITSELF an `Instance` of this SAME type —
                                // i.e. unambiguously "some OTHER instance of
                                // the identical class", the exact cross-
                                // instance-pollution case above. A cached
                                // method deliberately bound to something
                                // ELSE (native-backing delegation for a
                                // class transparently subclassing list/
                                // dict/str, or any other legitimate "bound
                                // to a fixed, different object" case) must
                                // be returned UNCHANGED — rebinding it
                                // unconditionally to `obj` broke exactly
                                // that (confirmed regression: `collections`'
                                // own `Counter.update` internals, which rely
                                // on a cached method staying bound to its
                                // real native-backing dict rather than the
                                // wrapper `Instance`).
                                let rebound = match &*cached_val.borrow() {
                                    PyObject::BuiltinMethod {
                                        name: n,
                                        func,
                                        self_obj,
                                    } if matches!(&*self_obj.borrow(), PyObject::Instance { typ: t, .. } if t.is(typ)) => {
                                        PyObjectRef::imm(PyObject::BuiltinMethod {
                                            name: n.clone(),
                                            func: *func,
                                            self_obj: obj.clone(),
                                        })
                                    }
                                    PyObject::BoundMethod { func, self_obj } if matches!(&*self_obj.borrow(), PyObject::Instance { typ: t, .. } if t.is(typ)) => {
                                        PyObjectRef::imm(PyObject::BoundMethod {
                                            func: func.clone(),
                                            self_obj: obj.clone(),
                                        })
                                    }
                                    _ => cached_val.clone(),
                                };
                                self.frames[fi].push(rebound);
                                return Ok(None);
                            }
                            if name == "__dict__" {
                                // Return a live Dict view backed by the instance's HashMap.
                                // NATIVE_BACKING_KEY is internal bookkeeping
                                // (see native_backing_of) and must not leak
                                // into user-visible introspection.
                                let mut pd = crate::object::PyDict::new();
                                for (k, v) in dict.iter() {
                                    if k == crate::object::NATIVE_BACKING_KEY {
                                        continue;
                                    }
                                    let key = py_str(k);
                                    pd.set(key, v.clone())?;
                                }
                                drop(obj_borrowed);
                                pd.instance_ref = Some(obj.clone());
                                self.frames[fi]
                                    .push(PyObjectRef::new(PyObject::Dict(Box::new(pd))));
                                return Ok(None);
                            }
                            if name == "__class__" {
                                let cls = typ.clone();
                                drop(obj_borrowed);
                                self.frames[fi].push(cls);
                                return Ok(None);
                            }
                            // Clone dict/typ into owned values and drop the
                            // borrow of `obj` ITSELF now — the descriptor
                            // dispatch below may call into arbitrary Python
                            // code (a `@property` getter, `cached_property`'s
                            // `__get__`, etc.), and such code very commonly
                            // writes back into `instance.__dict__` (that's
                            // literally what `cached_property.__get__` does,
                            // to cache its computed value for next time) —
                            // if `obj`'s own borrow were still held here, that
                            // nested write's borrow_mut() on the SAME RefCell
                            // panics the moment such a getter touches the
                            // instance it was called on (confirmed via a
                            // genuine, general, Django-free repro).
                            let dict: crate::object::AttrMap = dict.clone();
                            let typ: PyObjectRef = typ.clone();
                            drop(obj_borrowed);
                            let dict = &dict;
                            let typ = &typ;
                            let attr = if let Some(inst_attr) = dict.get_str(&name) {
                                Ok(Some(inst_attr.clone()))
                            } else {
                                (|| -> PyResult<Option<PyObjectRef>> {
                                    let typ_ref = typ.borrow();
                                    if let PyObject::Type {
                                        dict: type_dict,
                                        mro,
                                        ..
                                    } = &*typ_ref
                                    {
                                        let found =
                                            type_dict.get_str(&name).cloned().or_else(|| {
                                                for base in mro.iter().skip(1) {
                                                    if let PyObject::Type {
                                                        dict: base_dict, ..
                                                    } = &*base.borrow()
                                                    {
                                                        if let Some(val) = base_dict.get_str(&name)
                                                        {
                                                            return Some(val.clone());
                                                        }
                                                    }
                                                }
                                                None
                                            });
                                        // Handle descriptor protocol for Property, StaticMethod, ClassMethod, and generic __get__
                                        if let Some(val) = found {
                                            let val_borrowed = val.borrow();
                                            match &*val_borrowed {
                                            PyObject::Property(ref d) if d.getter.is_some() => {
                                                drop(typ_ref);
                                                let g = d.getter.as_ref().unwrap();
                                                return Ok(Some(self.call_function(g.clone(), vec![obj.clone()], vec![]).unwrap_or_else(|_| val.clone())));
                                            }
                                            PyObject::StaticMethod { func } => {
                                                return Ok(Some(func.clone()));
                                            }
                                            PyObject::ClassMethod { func } => {
                                                let func_clone = func.clone();
                                                drop(val_borrowed);
                                                drop(typ_ref);
                                                let cls = obj.borrow();
                                                if let PyObject::Instance { typ: inst_typ, .. } = &*cls {
                                                    // Return a BoundMethod that will prepend the class when called
                                                    let class_obj = inst_typ.clone();
                                                    drop(cls);
                                                    return Ok(Some(PyObjectRef::imm(PyObject::BoundMethod {
                                                        func: func_clone,
                                                        self_obj: class_obj,
                                                    })));
                                                }
                                                // When accessing classmethod on a type itself (e.g. MyClass.method),
                                                // bind the type as self so it becomes the first arg on call
                                                let class_obj = obj.clone();
                                                drop(cls);
                                                return Ok(Some(PyObjectRef::imm(PyObject::BoundMethod {
                                                    func: func_clone,
                                                    self_obj: class_obj,
                                                })));
                                            }
                                            PyObject::Function(_) => {
                                                let is_instance_obj = matches!(&*obj.borrow(), PyObject::Instance { .. });
                                                if is_instance_obj {
                                                    return Ok(Some(PyObjectRef::imm(PyObject::BoundMethod {
                                                        func: val.clone(),
                                                        self_obj: obj.clone(),
                                                    })));
                                                } else {
                                                    return Ok(Some(val.clone()));
                                                }
                                            }
                                            PyObject::BuiltinFunction { name: n, func }
                                                if crate::object::is_builtin_exception_class_name(n)
                                                    || std::ptr::fn_addr_eq(*func, crate::object::builtin_open as crate::object::BuiltinFunc) =>
                                            {
                                                // Do NOT auto-bind a builtin
                                                // exception "class" (this
                                                // codebase's representation for
                                                // e.g. `AssertionError`) found
                                                // as a plain class attribute
                                                // (`failureException =
                                                // AssertionError`) — unlike a
                                                // genuine native METHOD (also a
                                                // `BuiltinFunction`, e.g.
                                                // `hmac`'s `HMAC.hexdigest`,
                                                // which DOES rely on `self`
                                                // being auto-prepended — see
                                                // the `else` arm just below,
                                                // unchanged for that case), a
                                                // class reference is never a
                                                // descriptor in real Python, so
                                                // binding it here silently
                                                // prepended `self` as an extra
                                                // positional argument to every
                                                // call and turned the class
                                                // reference into a `BoundMethod`
                                                // that `issubclass()` could no
                                                // longer recognize as a class at
                                                // all — confirmed via
                                                // `unittest`'s own
                                                // `self.failureException(msg)`
                                                // (raising `AssertionError(self,
                                                // msg)` instead of
                                                // `AssertionError(msg)`) and
                                                // `issubclass(exc_info[0],
                                                // test.failureException)`
                                                // (always False, misclassifying
                                                // every real test failure as an
                                                // error).
                                                let _ = func;
                                                return Ok(Some(val.clone()));
                                            }
                                            PyObject::BuiltinFunction { name: n, func } => {
                                                let n = n.clone();
                                                let func = *func;
                                                // Plain builtin module functions
                                                // (`isclose = math.isclose`) have no
                                                // `__get__` and must stay UNBOUND — a
                                                // genuinely bound native method lives
                                                // only in its type's dict, never in a
                                                // module namespace. Drop the borrow
                                                // first: if `val` IS a module member,
                                                // the scan below re-borrows the same
                                                // RefCell.
                                                drop(val_borrowed);
                                                if self.is_plain_module_function(&n, &val) {
                                                    return Ok(Some(val.clone()));
                                                }
                                                return Ok(Some(PyObjectRef::imm(PyObject::BuiltinMethod {
                                                    name: n,
                                                    func,
                                                    self_obj: obj.clone(),
                                                })));
                                            }
                                            PyObject::BuiltinMethod { name: n, func, .. } => {
                                                return Ok(Some(PyObjectRef::imm(PyObject::BuiltinMethod {
                                                    name: n.clone(),
                                                    func: *func,
                                                    self_obj: obj.clone(),
                                                })));
                                            }
                                            // NOTE: deliberately NOT auto-binding a bare
                                            // `PyObject::Closure` found via the class dict here
                                            // — unlike `Function`/`BuiltinFunction` just above,
                                            // `Closure` is ALSO used pervasively for the
                                            // opposite convention: a closure built FRESH per
                                            // instance (e.g. `io.BytesIO`'s `read`/`write`/
                                            // `seek`, `dev.rs`), capturing that instance's own
                                            // state directly and expecting NO `self` prepended
                                            // at all. Auto-binding unconditionally here broke
                                            // those (their first REAL argument became `self`
                                            // instead) — confirmed via `io.BytesIO().write(b"x")`
                                            // regressing to `TypeError: a bytes-like object is
                                            // required, not str`. A shared, TYPE-level `Closure`
                                            // needing `self` (e.g. `namedtuple`'s own
                                            // `_asdict`/`_replace`) should instead be
                                            // implemented as a plain `BuiltinFunction` reading
                                            // whatever state it needs off `self` at call time —
                                            // that convention already auto-binds correctly via
                                            // the arm above, with no ambiguity.
                                            _ => {
                                                // Generic descriptor protocol: if value has __get__, call it
                                                drop(val_borrowed);
                                                let cls = {
                                                    let owner_type = obj.borrow();
                                                    if let PyObject::Instance { typ: inst_typ, .. } = &*owner_type {
                                                        Some(inst_typ.clone())
                                                    } else {
                                                        None
                                                    }
                                                };
                                                if let Some(cls) = cls {
                                                    if let Ok(__get__) = val.borrow().get_attribute("__get__") {
                                                        if std::env::var("RPY_DEBUG_DESCRIPTOR2").is_ok() {
                                                            eprintln!("INSTANCE-LEVEL __get__: attr_name={} val_type={:?} obj_type={:?}", name, val.borrow().type_name(), obj.borrow().type_name());
                                                        }
                                                        let descriptor_args = vec![val.clone(), obj.clone(), cls];
                                                        match self.call_function(__get__, descriptor_args, vec![]) {
                                                            Ok(v) => return Ok(Some(v)),
                                                            Err(e) => return Err(e),
                                                        }
                                                    }
                                                }
                                                return Ok(Some(val.clone()));
                                            }
                                        }
                                        }
                                        Ok(None)
                                    } else {
                                        Ok(None)
                                    }
                                })()
                            };
                            let attr = attr?;
                            // Not overridden anywhere in the mro: for a class
                            // that transparently subclasses list/dict/str
                            // (`class Foo(list): ...`), delegate to the real
                            // native value's own attribute resolution, rebound
                            // to the native backing (not this instance) since
                            // that's the object whose state actually mutates.
                            // Must run BEFORE the generic dict-like fallback
                            // below, which would otherwise misinterpret the
                            // native backing's own dict entry as plain
                            // instance-attribute data.
                            let attr = attr.or_else(|| {
                                let native = dict.get(crate::object::NATIVE_BACKING_KEY)?;
                                // A deque subclass's `__copy__`/`copy()` must
                                // return a NEW instance of the SAME subclass
                                // (not a raw deque) — build that closure here,
                                // since this inline resolution path mirrors
                                // `get_attribute_impl`'s own handling.
                                if matches!(&*native.borrow(), PyObject::Deque { .. })
                                    && (name == "__copy__" || name == "copy")
                                {
                                    let typ_clone = typ.clone();
                                    let new_native = {
                                        let b = native.borrow();
                                        if let PyObject::Deque { data, maxlen } = &*b {
                                            py_deque(data.clone(), *maxlen)
                                        } else {
                                            unreachable!()
                                        }
                                    };
                                    return Some(PyObjectRef::new(PyObject::Closure(Rc::new(
                                        move |_args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                                            let mut new_dict = crate::object::AttrMap::new();
                                            new_dict.insert(
                                                crate::object::NATIVE_BACKING_KEY.to_string(),
                                                new_native.clone(),
                                            );
                                            Ok(PyObjectRef::new(PyObject::Instance {
                                                typ: typ_clone.clone(),
                                                dict: new_dict,
                                            }))
                                        },
                                    ))));
                                }
                                let val = native.borrow().get_attribute(&name).ok()?;
                                let rebound = match &*val.borrow() {
                                    PyObject::BuiltinMethod { name: n, func, .. } => {
                                        Some(PyObjectRef::imm(PyObject::BuiltinMethod {
                                            name: n.clone(),
                                            func: *func,
                                            self_obj: native.clone(),
                                        }))
                                    }
                                    _ => None,
                                };
                                Some(rebound.unwrap_or(val))
                            });
                            // Fallback for dict methods on dict-derived instances
                            let attr = attr.or_else(|| {
                                if name == "__iter__"
                                    || name == "items"
                                    || name == "keys"
                                    || name == "values"
                                    || name == "get"
                                {
                                    let func: crate::object::BuiltinFunc = match name.as_str() {
                                        "__iter__" => crate::object::dict_method_iter,
                                        "items" => crate::object::dict_method_items,
                                        "keys" => crate::object::dict_method_keys,
                                        "values" => crate::object::dict_method_values,
                                        "get" => crate::object::dict_method_get,
                                        _ => return None,
                                    };
                                    Some(PyObjectRef::imm(PyObject::BuiltinMethod {
                                        name: name.clone(),
                                        func,
                                        self_obj: obj.clone(),
                                    }))
                                } else {
                                    None
                                }
                            });
                            // PEP 3134 traceback/chaining protocol methods
                            // for a user-defined exception class that
                            // doesn't override them — same fix, same
                            // rationale, as the `get_attribute_impl` copy of
                            // this logic (`object.rs`); this is LOAD_ATTR's
                            // own separate, inline copy of instance
                            // attribute resolution (kept for its attribute
                            // cache), which needs the identical fallback.
                            let attr = attr.or_else(|| {
                                if matches!(name.as_str(), "with_traceback" | "add_note" | "__traceback__" | "__context__" | "__cause__" | "__suppress_context__" | "__notes__")
                                    && crate::object::find_exception_base_name(typ).is_some() {
                                    Some(match name.as_str() {
                                        "with_traceback" => PyObjectRef::imm(PyObject::BuiltinMethod {
                                            name: "with_traceback".to_string(),
                                            func: |args| {
                                                if args.is_empty() { return Err(PyError::type_error("with_traceback() takes exactly one argument")); }
                                                Ok(args[0].clone())
                                            },
                                            self_obj: obj.clone(),
                                        }),
                                        "add_note" => PyObjectRef::imm(PyObject::BuiltinMethod {
                                            name: "add_note".to_string(),
                                            func: |_args| Ok(py_none()),
                                            self_obj: obj.clone(),
                                        }),
                                        // See the matching fix (and its full
                                        // explanation) in `get_attribute_impl`'s
                                        // copy of this same list (`attrs.rs`) —
                                        // `__cause__` was missing from both.
                                        "__context__" | "__traceback__" | "__cause__" => py_none(),
                                        "__suppress_context__" => py_bool(false),
                                        "__notes__" => py_list(vec![]),
                                        _ => unreachable!(),
                                    })
                                } else {
                                    None
                                }
                            });
                            match attr {
                                Some(val) => {
                                    // Cache attribute for future accesses — but
                                    // ONLY when it was found on the TYPE's own
                                    // dict/mro (a method or class attribute,
                                    // identical for every instance of this
                                    // type), never when it came from the
                                    // INSTANCE's own dict. A plain instance
                                    // attribute (`self.v`) varies per-instance,
                                    // but this cache is keyed only by
                                    // `(name_idx, type_tag)` — with no
                                    // per-instance component at all — so
                                    // caching an instance-dict hit here meant
                                    // ANY second instance of the same type
                                    // accessed via the same attribute name
                                    // within the same frame (e.g. `self.v` vs
                                    // `other.v` inside `__lt__(self, other)`)
                                    // silently got back the FIRST instance's
                                    // value instead of its own — a severe,
                                    // general correctness bug, not merely a
                                    // missed-cache-hit inefficiency. Confirmed
                                    // via a minimal repro: `other.v` returning
                                    // `self.v`'s value inside a two-argument
                                    // comparison method.
                                    //
                                    // A `property`'s (or any other `__get__`-
                                    // based descriptor's) getter is called
                                    // ABOVE and only its RETURN VALUE reaches
                                    // this point — that value is exactly as
                                    // instance-specific as a plain instance
                                    // attribute (it's computed FROM the
                                    // instance's own state), so caching it
                                    // here under the same "found on the type"
                                    // reasoning is the identical bug in a
                                    // different disguise: every instance of
                                    // the class sharing this one cache slot
                                    // got back the FIRST instance's computed
                                    // value forever after. Confirmed via a
                                    // minimal, `__slots__`-free repro: `class
                                    // Foo: x = property(lambda self: self.v)`
                                    // — `b.x` returned `a.x`'s value.
                                    let is_property_result = {
                                        let typ_ref = typ.borrow();
                                        if let PyObject::Type {
                                            dict: type_dict,
                                            mro,
                                            ..
                                        } = &*typ_ref
                                        {
                                            let found_val: Option<PyObjectRef> =
                                                type_dict.get_str(&name).cloned().or_else(|| {
                                                    mro.iter().skip(1).find_map(|base| {
                                                        if let PyObject::Type {
                                                            dict: base_dict,
                                                            ..
                                                        } = &*base.borrow()
                                                        {
                                                            base_dict.get_str(&name).cloned()
                                                        } else {
                                                            None
                                                        }
                                                    })
                                                });
                                            found_val
                                                .map(|v| {
                                                    matches!(&*v.borrow(), PyObject::Property(_))
                                                })
                                                .unwrap_or(false)
                                        } else {
                                            false
                                        }
                                    };
                                    // A method bound to THIS instance's own
                                    // native backing (deque subclass: `pop`/
                                    // `append`/... resolved via the native
                                    // delegation at `get_attribute_impl`) is
                                    // inherently per-instance — caching it
                                    // under a `(name_idx, type_tag)` key with
                                    // no per-instance component means the NEXT
                                    // instance of the same class in this frame
                                    // silently reuses a method still bound to
                                    // the FIRST instance's backing and mutates
                                    // the wrong object (confirmed via a deque
                                    // subclass's `d.pop(); e.pop()` in one
                                    // frame). `PyObject::Closure` values are
                                    // excluded for the same reason: a
                                    // per-instance closure (e.g. a deque
                                    // subclass's `__copy__`, which captures
                                    // that instance's own backing) must not
                                    // leak into a cache another instance
                                    // reuses.
                                    let is_native_backing_bound = matches!(&*val.borrow(), PyObject::BuiltinMethod { self_obj, .. }
                                        if !matches!(&*self_obj.borrow(), PyObject::Instance { .. } | PyObject::None))
                                        || matches!(&*val.borrow(), PyObject::Closure(_));
                                    if !dict.contains_key(&name)
                                        && !is_property_result
                                        && !is_native_backing_bound
                                        && name_idx < self.frames[fi].attr_cache.len()
                                    {
                                        self.frames[fi].attr_cache[name_idx] =
                                            Some((type_tag, val.clone()));
                                    }
                                    Ok(val)
                                }
                                None => {
                                    // Check for __getattr__ before erroring —
                                    // via the full mro, not just typ's own
                                    // dict: `__getattr__` is very commonly
                                    // defined on a BASE class (e.g. Django's
                                    // `LazyObject.__getattr__ =
                                    // new_method_proxy(getattr)`, inherited
                                    // by `SimpleLazyObject`) rather than
                                    // redeclared on every subclass, and the
                                    // instance's own exact class rarely
                                    // defines it directly.
                                    let getattr_method =
                                        crate::object::lookup_dunder_via_mro(typ, "__getattr__");
                                    if let Some(getattr_method) = getattr_method {
                                        self.call_function(
                                            getattr_method,
                                            vec![obj.clone(), py_str(&name)],
                                            vec![],
                                        )
                                    } else if name == "__doc__" {
                                        // Every real object has __doc__ (defaults to
                                        // None) — see the matching fallback in
                                        // object.rs's ObjectAccess::get_attribute.
                                        Ok(py_none())
                                    } else {
                                        // Attach name/obj to the AttributeError
                                        // (CPython: `exc.name`/`exc.obj` after
                                        // `obj.missing_attr`).
                                        let mut extra = std::collections::HashMap::new();
                                        extra.insert("name".to_string(), py_str(&name));
                                        extra.insert("obj".to_string(), obj.clone());
                                        Err(PyError::Exception(
                                            "AttributeError".to_string(),
                                            PyObjectRef::new(PyObject::Exception {
                                                typ: "AttributeError".to_string(),
                                                args: vec![py_str(&format!(
                                                    "'{}' object has no attribute '{}'",
                                                    crate::object::get_type_name_for_instance(typ),
                                                    name
                                                ))],
                                                cause: None,
                                                suppress_context: false,
                                                context: None,
                                                traceback: None,
                                                extra: Some(extra),
                                            }),
                                        ))
                                    }
                                }
                            }
                        }
                        _ => {
                            let type_name = obj_borrowed.type_name();
                            // Check inline cache first — but NOT for a TYPE
                            // receiver: the cache is keyed only by
                            // `(type_name, name)` and is populated by
                            // VALUE-level lookups (`line.strip` on a str
                            // value caches the instance strip under
                            // ("str","strip")), so a later TYPE-level lookup
                            // (`str.strip` on the str CLASS) would hit it and
                            // return the WRONG, already-value-bound method
                            // bound to the class (str.strip(s) then passed
                            // the class as self -> "<class 'str'>"). Type
                            // attribute access is rare and needs the full
                            // get_attribute path (which returns the raw
                            // unbound descriptor).
                            let is_type_obj = matches!(&*obj_borrowed, PyObject::Type { .. });
                            let is_globals = matches!(&*obj_borrowed, PyObject::Globals(_));
                            let cached = if is_type_obj || is_globals {
                                None
                            } else {
                                ATTR_CACHE.with(|c| {
                                    c.borrow().get(&(type_name.clone(), name.clone())).copied()
                                })
                            };
                            if let Some(func) = cached {
                                drop(obj_borrowed);
                                Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                                    name: name.clone(),
                                    func,
                                    self_obj: obj.clone(),
                                }))
                            } else {
                                let direct = obj_borrowed.get_attribute(&name);
                                let obj_type_name_for_err = obj_borrowed.type_name();
                                let attr = match direct {
                                    Ok(v) => v,
                                    Err(_) => {
                                        // Metaclass attribute fallback: a
                                        // class-level attribute not found on
                                        // the class's own dict/mro may still
                                        // exist on its *metaclass* (e.g. a
                                        // `@property` defined on a custom
                                        // metaclass like Django's
                                        // `ChoicesType.choices` — meant to be
                                        // read as `SomeChoicesClass.choices`,
                                        // with the class itself as the
                                        // property's "self"). Ordinary
                                        // classes have no METATYPE_KEY set,
                                        // so this is a no-op for them.
                                        let metatype_hit = if is_type_obj {
                                            crate::object::metatype_of(&obj).and_then(|mt| {
                                                if let PyObject::Type { mro, .. } = &*mt.borrow() {
                                                    for base in mro.iter() {
                                                        if let PyObject::Type { dict, .. } =
                                                            &*base.borrow()
                                                        {
                                                            if let Some(val) = dict.get_str(&name) {
                                                                return Some(val.clone());
                                                            }
                                                        }
                                                    }
                                                }
                                                None
                                            })
                                        } else {
                                            None
                                        };
                                        match metatype_hit {
                                            Some(val) => {
                                                let is_property = if let PyObject::Property(ref d) =
                                                    &*val.borrow()
                                                {
                                                    d.getter.is_some()
                                                } else {
                                                    false
                                                };
                                                if is_property {
                                                    let getter = if let PyObject::Property(ref d) =
                                                        &*val.borrow()
                                                    {
                                                        d.getter.clone().unwrap()
                                                    } else {
                                                        unreachable!()
                                                    };
                                                    drop(obj_borrowed);
                                                    let result = self.call_function(
                                                        getter,
                                                        vec![obj.clone()],
                                                        vec![],
                                                    )?;
                                                    self.frames[fi].push(result);
                                                    return Ok(None);
                                                }
                                                // `obj` (a class) is being accessed as an
                                                // INSTANCE of its own metaclass here (that's
                                                // what "found on the metatype, not on obj's
                                                // own dict/mro" means) — an ordinary method
                                                // found this way must auto-bind `self=obj`,
                                                // exactly like any instance accessing a
                                                // regular method, or its first real
                                                // parameter (e.g. Django's metaclass method
                                                // `add_to_class(cls, name, value)`, called as
                                                // `new_class.add_to_class(name, value)`) never
                                                // gets bound and every later positional arg
                                                // silently shifts left by one. This is
                                                // distinct from ordinary `SomeClass.method`
                                                // access (the `is_function => Ok(attr)` case
                                                // below), which correctly stays unbound.
                                                if matches!(&*val.borrow(), PyObject::Function(_)) {
                                                    drop(obj_borrowed);
                                                    self.frames[fi].push(PyObjectRef::imm(
                                                        PyObject::BoundMethod {
                                                            func: val,
                                                            self_obj: obj.clone(),
                                                        },
                                                    ));
                                                    return Ok(None);
                                                }
                                                val
                                            }
                                            None => {
                                                drop(obj_borrowed);
                                                if name == "__doc__" {
                                                    self.frames[fi].push(py_none());
                                                    return Ok(None);
                                                }
                                                // PEP 562 module `__getattr__`:
                                                // a module whose own dict lacks
                                                // the attribute but defines
                                                // `__getattr__` (lazy-attribute
                                                // pattern; real trigger: the
                                                // vendored `test.support.
                                                // _hypothesis_stubs.strategies`,
                                                // which backends every
                                                // strategy name through a
                                                // module-level `__getattr__`)
                                                // must have it invoked before
                                                // erroring.
                                                if matches!(&*obj.borrow(), PyObject::Module { .. })
                                                {
                                                    let g =
                                                        obj.borrow().get_attribute("__getattr__");
                                                    if let Ok(getattr_method) = g {
                                                        if !matches!(
                                                            &*getattr_method.borrow(),
                                                            PyObject::None
                                                        ) {
                                                            // Module __getattr__ takes ONLY the attribute name —
                                                            // the module itself is NOT bound as self.
                                                            let result = self.call_function(
                                                                getattr_method,
                                                                vec![py_str(&name)],
                                                                vec![],
                                                            )?;
                                                            self.frames[fi].push(result);
                                                            return Ok(None);
                                                        }
                                                    }
                                                }
                                                return Err(PyError::attribute_error(format!(
                                                    "'{}' object has no attribute '{}'",
                                                    obj_type_name_for_err, name
                                                )));
                                            }
                                        }
                                    }
                                };
                                drop(obj_borrowed);
                                // Generic descriptor protocol for class-level
                                // attribute access (`Foo.attr`, `obj` here is
                                // the type itself): a plain user-defined
                                // descriptor class (any Instance whose type
                                // defines __get__ — e.g. Django's
                                // class_or_instance_method) must have __get__
                                // invoked with instance=None, matching the
                                // generic __get__ handling already done for
                                // instance-level access above. Builtin
                                // Property/StaticMethod/ClassMethod/Function
                                // descriptors are already special-cased below
                                // and are never PyObject::Instance, so this
                                // can't double-invoke them.
                                if is_type_obj {
                                    if matches!(&*attr.borrow(), PyObject::Instance { .. }) {
                                        if let Ok(get_fn) = attr.borrow().get_attribute("__get__") {
                                            if std::env::var("RPY_DEBUG_DESCRIPTOR2").is_ok() {
                                                eprintln!("CLASS-LEVEL __get__: attr_name={} obj_type={:?}", name, obj.borrow().type_name());
                                            }
                                            let result = self.call_function(
                                                get_fn,
                                                vec![attr.clone(), py_none(), obj.clone()],
                                                vec![],
                                            )?;
                                            self.frames[fi].push(result);
                                            return Ok(None);
                                        }
                                    }
                                }
                                // Resolve classmethod descriptor for type attribute access
                                {
                                    let ab = attr.borrow();
                                    if let PyObject::ClassMethod { func } = &*ab {
                                        let func_clone = func.clone();
                                        let cls_obj = obj.clone();
                                        drop(ab);
                                        let bound = PyObjectRef::new(PyObject::BoundMethod {
                                            func: func_clone,
                                            self_obj: cls_obj,
                                        });
                                        self.frames[fi].push(bound);
                                        return Ok(None);
                                    }
                                }
                                // Only rebind self_obj (and cache the
                                // func-pointer fast path keyed on it) when
                                // the found `BuiltinMethod`'s own self_obj is
                                // still the `PyObject::None` PLACEHOLDER —
                                // the established convention native
                                // container methods (File/List/Dict/Set/
                                // frozenset's own `.append`/`.get`/etc.) use,
                                // meaning "rebind me to whatever object I
                                // was actually looked up on". A
                                // BuiltinMethod that's already bound to some
                                // OTHER real object (e.g. a MODULE-level
                                // `iskeyword = frozenset(kwlist).__contains__`
                                // — self_obj is that frozenset, permanently,
                                // and `obj` here is the *module* being
                                // accessed as `keyword.iskeyword`) must be
                                // returned completely unchanged. Previously
                                // this unconditionally rebuilt EVERY
                                // BuiltinMethod found this way with
                                // `self_obj: obj.clone()`, discarding the
                                // real target and substituting the
                                // currently-accessed object instead —
                                // confirmed general via `import keyword;
                                // keyword.iskeyword("if")` raising
                                // `RuntimeError: __contains__ on
                                // non-frozenset` (self_obj had silently
                                // become the `keyword` module itself).
                                let is_placeholder_self = matches!(&*attr.borrow(), PyObject::BuiltinMethod { self_obj, .. } if matches!(&*self_obj.borrow(), PyObject::None));
                                let is_function = matches!(&*attr.borrow(), PyObject::Function(_));
                                if is_placeholder_self {
                                    let (n, func) = {
                                        let b = attr.borrow();
                                        if let PyObject::BuiltinMethod { name: n, func, .. } = &*b {
                                            (n.clone(), *func)
                                        } else {
                                            unreachable!()
                                        }
                                    };
                                    // Cache for next time — but NOT
                                    // `__init__` (nor `__new__`): a native
                                    // VALUE's `__init__` (e.g. a raw deque's,
                                    // resolved via `attrs.rs`'s per-value arm)
                                    // and the same-name TYPE-level attribute
                                    // (`deque.__init__`, the native-base
                                    // initializer) are DIFFERENT methods, yet
                                    // this cache is keyed only by
                                    // `(type_name, name)` — caching the
                                    // value-level one made `deque.__init__`
                                    // silently return the wrong function after
                                    // any `d.__init__(...)` call.
                                    if n != "__init__"
                                        && n != "__new__"
                                        && !matches!(&*obj.borrow(), PyObject::Globals(_))
                                    {
                                        ATTR_CACHE.with(|c| {
                                            c.borrow_mut()
                                                .insert((type_name.clone(), n.clone()), func);
                                        });
                                    }
                                    Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                                        name: n,
                                        func,
                                        self_obj: obj.clone(),
                                    }))
                                } else if is_function {
                                    Ok(attr)
                                } else {
                                    Ok(attr)
                                }
                            }
                        }
                    }
                }?;
                self.frames[fi].push(result);
            }

            Opcode::STORE_ATTR => {
                let name_idx = arg as usize;
                let name = crate::interner::lookup(self.frames[fi].code.names[name_idx]);
                let val = self.frames[fi].pop()?;
                let obj = self.frames[fi].pop()?;
                if std::env::var("RPY_DEBUG_ATTR").is_ok() {
                    let kind = match &*obj.borrow() {
                        PyObject::Type { name: n, .. } => format!("Type({})", n),
                        PyObject::Module { name: n, .. } => format!("Module({})", n),
                        PyObject::Instance { .. } => "Instance".to_string(),
                        other => format!("{:?}", std::mem::discriminant(other)),
                    };
                    eprintln!("STORE_ATTR: name={} obj_kind={}", name, kind);
                }

                // Check for __setattr__ on Instance types first
                {
                    let obj_borrowed = obj.borrow();
                    if let PyObject::Instance { typ, .. } = &*obj_borrowed {
                        let typ_ref = typ.borrow();
                        if let PyObject::Type {
                            dict: type_dict, ..
                        } = &*typ_ref
                        {
                            if let Some(setattr_method) = type_dict.get_str("__setattr__").cloned()
                            {
                                drop(typ_ref);
                                drop(obj_borrowed);
                                // Call __setattr__ for side effects (validation, clearing caches)
                                let result = self.call_function(
                                    setattr_method,
                                    vec![obj.clone(), py_str(&name), val.clone()],
                                    vec![],
                                );
                                // Also set the attribute directly in the instance dict, since
                                // __dict__ returns a COPY and self.__dict__[key] = value inside
                                // __setattr__ would modify the copy, not the original.
                                if let PyObject::Instance { dict, .. } = &mut *obj.borrow_mut() {
                                    dict.insert_str(&name, val.clone());
                                }
                                result?;
                                return Ok(None);
                            }
                        }
                    }
                }

                // Check for __set__ descriptor protocol on Instance types
                let descriptor_clone = {
                    let obj_borrowed = obj.borrow();
                    if let PyObject::Instance { typ, .. } = &*obj_borrowed {
                        let typ_ref = typ.borrow();
                        if let PyObject::Type {
                            dict: type_dict, ..
                        } = &*typ_ref
                        {
                            type_dict.get_str(&name).cloned()
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                };
                if let Some(descriptor) = descriptor_clone {
                    // Property is special-cased directly (matching how LOAD_ATTR's
                    // getter path already calls `self.call_function` on the real
                    // getter function directly, not through a wrapper) instead of
                    // going through the generic `get_attribute("__set__")` below.
                    // That generic path returns a `BuiltinMethod` whose closure
                    // body calls the free `call_bound_method` function internally
                    // (a plain `fn(&[PyObjectRef])` has no `&mut VirtualMachine`
                    // to call through) — which spins up a disposable VM with an
                    // empty module registry. A property setter that does a lazy
                    // `import` internally (a real, common Django pattern used
                    // specifically to sidestep circular imports) would then
                    // re-import everything from scratch in that disposable VM
                    // instead of seeing what's already loaded.
                    let property_setter = {
                        let d = descriptor.borrow();
                        if let PyObject::Property(ref data) = &*d {
                            data.setter.clone()
                        } else {
                            None
                        }
                    };
                    if let Some(setter_fn) = property_setter {
                        self.call_function(setter_fn, vec![obj.clone(), val.clone()], vec![])?;
                        return Ok(None);
                    }
                    let setter_method = { descriptor.borrow().get_attribute("__set__").ok() };
                    if let Some(setter_method) = setter_method {
                        let result = self.call_function(
                            setter_method,
                            vec![descriptor, obj.clone(), val.clone()],
                            vec![],
                        );
                        match result {
                            Ok(_) => return Ok(None),
                            Err(e) => return Err(e),
                        }
                    } else {
                        // Descriptor exists but no __set__ (non-data descriptor), fall through
                    }
                }
                // `obj.borrow_mut()` panics unconditionally for any
                // non-`Mut`-wrapped value (SmallInt/SmallBool/SmallFloat/
                // SmallStr/None, or an `Imm`-wrapped Tuple/Bytes/Function/
                // Type/Code/boxed-Int/Str/Float) — genuinely attribute-
                // settable things (Instance, Type, Module, Exception) are
                // ALWAYS `Mut` in this codebase, so anything reaching here
                // that ISN'T `Mut` is a real attempt to set an attribute on
                // an immutable/inline value (`(5).x = 1`, `"s".x = 1`,
                // `(1, 2).x = 1`) — real CPython raises a plain
                // `AttributeError` there, not a process-ending crash. This
                // was one of the highest-impact bugs found this session:
                // it crashed the WHOLE interpreter process (not just the
                // current statement) for something this common — including
                // every test file that deliberately checks this raises via
                // `self.assertRaises(AttributeError, setattr, x, 'attr', v)`.
                if !matches!(&obj, PyObjectRef::Mut(_)) {
                    return Err(PyError::attribute_error(format!(
                        "'{}' object has no attribute '{}'",
                        obj.borrow().type_name(),
                        name
                    )));
                }
                obj.borrow_mut().set_attribute(&name, val)?;
            }

            Opcode::STORE_SUBSCR => {
                let val = self.frames[fi].pop()?;
                let index = self.frames[fi].pop()?;
                let obj = self.frames[fi].pop()?;
                // If `obj` is an Instance with a Python-defined __setitem__,
                // call it via `self.call_function` (the real, already-live
                // VM) rather than falling into the free `py_setitem`
                // function's own Instance-dispatch, which calls it via
                // `call_bound_method` — a separate, pre-existing, documented
                // limitation that spins up a brand-new disposable
                // `VirtualMachine::new()` for the call. That's merely
                // wasteful for most code, but genuinely catastrophic for
                // any dict-subclass with a custom `__setitem__` used during
                // this VM's own construction (e.g. enum's `_EnumDict`,
                // whose `EnumType.__new__` does `namespace[key] = ...`) —
                // the disposable VM's construction re-runs the same
                // stdlib bootstrap, hits the same assignment again, and
                // recurses without end (confirmed via gdb backtrace).
                // Falls back to the free function for everything else
                // (native list/dict/tuple assignment, or an Instance with
                // no override delegating to its native backing), which
                // needs no VM access at all.
                let setitem_fn = if let PyObject::Instance { typ, .. } = &*obj.borrow() {
                    crate::object::lookup_dunder_via_mro(typ, "__setitem__")
                } else {
                    None
                };
                if let Some(f) = setitem_fn {
                    self.call_function(f, vec![obj.clone(), index, val], vec![])?;
                } else {
                    py_setitem(&obj, &index, val)?;
                }
            }

            Opcode::DELETE_SUBSCR => {
                let index = self.frames[fi].pop()?;
                let obj = self.frames[fi].pop()?;
                py_delitem(&obj, &index)?;
            }

            Opcode::DELETE_ATTR => {
                let name_idx = arg as usize;
                let name = crate::interner::lookup(self.frames[fi].code.names[name_idx]);
                let obj = self.frames[fi].pop()?;
                // Check for __delattr__ on Instance types first
                {
                    let obj_borrowed = obj.borrow();
                    if let PyObject::Instance { typ, .. } = &*obj_borrowed {
                        let typ_ref = typ.borrow();
                        if let PyObject::Type {
                            dict: type_dict, ..
                        } = &*typ_ref
                        {
                            if let Some(delattr_method) = type_dict.get_str("__delattr__").cloned()
                            {
                                drop(typ_ref);
                                drop(obj_borrowed);
                                self.call_function(
                                    delattr_method,
                                    vec![obj.clone(), py_str(&name)],
                                    vec![],
                                )?;
                                return Ok(None);
                            }
                        }
                    }
                }
                // Check for __delete__ descriptor protocol
                let descriptor = {
                    let obj_borrowed = obj.borrow();
                    if let PyObject::Instance { typ, .. } = &*obj_borrowed {
                        let typ_ref = typ.borrow();
                        if let PyObject::Type {
                            dict: type_dict, ..
                        } = &*typ_ref
                        {
                            type_dict.get_str(&name).cloned()
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                };
                if let Some(ref desc) = descriptor {
                    if let Ok(deleter) = desc.borrow().get_attribute("__delete__") {
                        let result =
                            self.call_function(deleter, vec![desc.clone(), obj.clone()], vec![]);
                        match result {
                            Ok(_) => return Ok(None),
                            Err(e) => return Err(e),
                        }
                    }
                }
                // `.borrow_mut()` panics unconditionally for anything that
                // ISN'T `PyObjectRef::Mut` — every inline variant plus any
                // `Imm`-wrapped value (boxed Int, Range, Tuple, Str, ...).
                // `del some_immutable_value.attr` (real trigger: CPython's
                // own `test_range.py`, `del rangeobj.start` — a `range`
                // object's `start`/`stop`/`step` are read-only, expected to
                // raise a clean `AttributeError`) previously panicked the
                // whole process instead. Same fix shape as `builtin_setattr`
                // already applies for `setattr()`.
                if !matches!(obj, PyObjectRef::Mut(_)) {
                    return Err(PyError::attribute_error(format!(
                        "'{}' object attribute '{}' is read-only",
                        obj.borrow().type_name(),
                        name
                    )));
                }
                obj.borrow_mut().del_attribute(&name)?;
            }

            Opcode::LIST_APPEND => {
                let val = self.frames[fi].pop()?;
                let list = self.frames[fi].peek(arg as usize)?;
                let mut obj = list.borrow_mut();
                if let PyObject::List(v) = &mut *obj {
                    v.push(val);
                } else {
                    return Err(PyError::runtime_error("LIST_APPEND on non-list"));
                }
            }

            Opcode::LIST_EXTEND => {
                let val = self.frames[fi].pop()?;
                let items: Vec<PyObjectRef> = {
                    let val_ref = val.borrow();
                    match &*val_ref {
                        PyObject::List(v) => v.clone(),
                        PyObject::Tuple(v) => v.clone(),
                        _ => {
                            drop(val_ref);
                            let iterator =
                                crate::object::builtin_iter(&[val.clone()]).map_err(|e| {
                                    // `*todata` unpacking / `list.extend(x)` on a
                                    // non-iterable raises TypeError, not RuntimeError
                                    // (difflib's HtmlDiff._collect_lines catches
                                    // TypeError for separator None values).
                                    PyError::type_error(e.to_string())
                                })?;
                            let mut result = Vec::new();
                            loop {
                                match crate::object::builtin_next(&[iterator.clone()]) {
                                    Ok(item) => result.push(item),
                                    Err(e) if crate::object::is_stop_iteration_error(&e) => break,
                                    Err(e) => return Err(e),
                                }
                            }
                            result
                        }
                    }
                };
                let list = self.frames[fi].peek(arg as usize)?;
                let mut obj = list.borrow_mut();
                if let PyObject::List(v) = &mut *obj {
                    v.extend(items);
                } else {
                    return Err(PyError::runtime_error("LIST_EXTEND on non-list"));
                }
            }

            Opcode::SET_ADD => {
                let val = self.frames[fi].pop()?;
                let set = self.frames[fi].peek(arg as usize)?;
                let mut obj = set.borrow_mut();
                if let PyObject::Set(v) = &mut *obj {
                    v.add(val)?;
                } else {
                    return Err(PyError::runtime_error("SET_ADD on non-set"));
                }
            }

            Opcode::SET_UPDATE => {
                // Backs `{*a, *b}` set-unpacking display syntax. Unlike
                // LIST_EXTEND (which only accepts list/tuple), CPython's
                // SET_UPDATE accepts any iterable, so pull items through
                // the same builtin_iter/builtin_next protocol chain() uses
                // rather than requiring a concrete List/Tuple/Set.
                let val = self.frames[fi].pop()?;
                let mut items = Vec::new();
                let it = crate::object::builtin_iter(&[val])?;
                loop {
                    match crate::object::builtin_next(&[it.clone()]) {
                        Ok(v) => items.push(v),
                        Err(e) if crate::object::is_stop_iteration_error(&e) => break,
                        Err(e) => return Err(e),
                    }
                }
                let set = self.frames[fi].peek(arg as usize)?;
                let mut obj = set.borrow_mut();
                if let PyObject::Set(v) = &mut *obj {
                    for item in items {
                        v.add(item)?;
                    }
                } else {
                    return Err(PyError::runtime_error("SET_UPDATE on non-set"));
                }
            }

            Opcode::MAP_ADD => {
                let val = self.frames[fi].pop()?;
                let key = self.frames[fi].pop()?;
                let map = self.frames[fi].peek(arg as usize)?;
                let mut obj = map.borrow_mut();
                if let PyObject::Dict(d) = &mut *obj {
                    d.set(key, val)?;
                } else {
                    return Err(PyError::runtime_error("MAP_ADD on non-dict"));
                }
            }

            Opcode::DICT_MERGE => {
                let source = self.frames[fi].pop()?;
                let target = self.frames[fi].peek(arg as usize)?;
                let source_items = {
                    let src_borrowed = source.borrow();
                    match &*src_borrowed {
                        PyObject::Dict(d) => d.items(),
                        // General mapping (e.g. `types.MappingProxyType`):
                        // DICT_MERGE is emitted for `{**mapping}` / `f(**mapping)`
                        // and only handled bare Dicts before — any other mapping
                        // raised "cannot merge non-dict into dict". Pull the
                        // items via `keys()` + `__getitem__` (the established
                        // call_method_rebound path; the mappingproxy closures
                        // work with or without a bound self).
                        _ => {
                            let mut out: Vec<(PyObjectRef, PyObjectRef)> = Vec::new();
                            let keys_obj =
                                crate::object::call_method_rebound(self, &source, "keys", vec![])?;
                            if let PyObject::List(items) = &*keys_obj.borrow() {
                                let keys: Vec<PyObjectRef> = items.clone();
                                for k in keys {
                                    let v = crate::object::call_method_rebound(
                                        self,
                                        &source,
                                        "__getitem__",
                                        vec![k.clone()],
                                    )?;
                                    out.push((k, v));
                                }
                            }
                            out
                        }
                    }
                };
                let mut target_borrowed = target.borrow_mut();
                if let PyObject::Dict(td) = &mut *target_borrowed {
                    for (k, v) in source_items {
                        td.set(k, v)?;
                    }
                } else {
                    return Err(PyError::runtime_error("DICT_MERGE on non-dict"));
                }
            }

            Opcode::LIST_TO_TUPLE => {
                let list = self.frames[fi].pop()?;
                let items = match &*list.borrow() {
                    PyObject::List(v) => v.clone(),
                    _ => return Err(PyError::runtime_error("LIST_TO_TUPLE on non-list")),
                };
                self.frames[fi].push(PyObjectRef::imm(PyObject::Tuple(items)));
            }

            Opcode::UNPACK_SEQUENCE => {
                let count = arg as usize;
                let seq = self.frames[fi].pop()?;
                let list_items = {
                    let obj = seq.borrow();
                    match &*obj {
                        PyObject::List(v) | PyObject::Tuple(v) => Some(v.clone()),
                        _ => None,
                    }
                };
                // Any other iterable (generator, custom __iter__, set, dict
                // view, str, ...) — real Python's unpacking assignment
                // accepts anything iterable, not just list/tuple literally.
                // Confirmed missing via Django's real `for k, v in
                // some_dict.items():`-adjacent unpacking during
                // `django.setup()`.
                let items = match list_items {
                    Some(v) => v,
                    None => {
                        let iterator =
                            crate::object::builtin_iter(&[seq.clone()]).map_err(|_| {
                                PyError::type_error(format!(
                                    "cannot unpack non-iterable '{}' object",
                                    seq.borrow().type_name()
                                ))
                            })?;
                        let mut v = Vec::new();
                        loop {
                            match crate::object::builtin_next(&[iterator.clone()]) {
                                Ok(val) => v.push(val),
                                Err(e) if crate::object::is_stop_iteration_error(&e) => break,
                                Err(e) => return Err(e),
                            }
                        }
                        v
                    }
                };
                if items.len() != count {
                    // Match real CPython's exact wording (confirmed against
                    // a real interpreter) — the previous generic "cannot
                    // unpack N items into M values" message matched neither
                    // phrasing CPython actually uses.
                    return Err(PyError::value_error(if items.len() < count {
                        format!(
                            "not enough values to unpack (expected {}, got {})",
                            count,
                            items.len()
                        )
                    } else {
                        format!("too many values to unpack (expected {})", count)
                    }));
                }
                for item in items.into_iter().rev() {
                    self.frames[fi].push(item);
                }
            }

            Opcode::UNPACK_EX => {
                let before = (arg >> 8) as usize;
                let after = (arg & 0xFF) as usize;
                let total = before + after + 1; // +1 for the starred item
                let seq = self.frames[fi].pop()?;
                let list_items = {
                    let obj = seq.borrow();
                    match &*obj {
                        PyObject::List(v) | PyObject::Tuple(v) => Some(v.clone()),
                        _ => None,
                    }
                };
                // Same generalization as UNPACK_SEQUENCE above: fall back to
                // the real iterator protocol for anything that isn't
                // literally a list/tuple.
                let items = match list_items {
                    Some(v) => v,
                    None => {
                        let iterator =
                            crate::object::builtin_iter(&[seq.clone()]).map_err(|_| {
                                PyError::type_error(format!(
                                    "cannot unpack non-iterable '{}' object",
                                    seq.borrow().type_name()
                                ))
                            })?;
                        let mut v = Vec::new();
                        loop {
                            match crate::object::builtin_next(&[iterator.clone()]) {
                                Ok(val) => v.push(val),
                                Err(e) if crate::object::is_stop_iteration_error(&e) => break,
                                Err(e) => return Err(e),
                            }
                        }
                        v
                    }
                };
                if items.len() < before + after {
                    // Matches real CPython's wording for starred unpacking
                    // (`a, *b, c = seq`) — "at least" since a starred target
                    // can absorb any number of extra items above the
                    // minimum (`before + after`), unlike plain
                    // `UNPACK_SEQUENCE`'s exact-count requirement.
                    return Err(PyError::value_error(format!(
                        "not enough values to unpack (expected at least {}, got {})",
                        before + after,
                        items.len()
                    )));
                }
                let _ = total;
                let n = items.len();
                // Push order (bottom of stack = first to be stored):
                //   before items, star list, after items
                // So we push in reverse: after items first (on top), then star list, then before items
                // Push after-star items (last N) in reverse
                for i in (n - after)..n {
                    self.frames[fi].push(items[i].clone());
                }
                // Push starred item (everything between before and after) as a list
                let star_count = n - before - after;
                let mut star_items: Vec<PyObjectRef> = Vec::new();
                for i in before..(before + star_count) {
                    star_items.push(items[i].clone());
                }
                self.frames[fi].push(py_list(star_items));
                // Push before-star items (first N) in reverse so first comes out on bottom
                for i in (0..before).rev() {
                    self.frames[fi].push(items[i].clone());
                }
            }

            Opcode::SETUP_FINALLY => {
                let stack_depth = self.frames[fi].stack.len();
                let handler = ExceptionHandler {
                    instr_addr: arg as usize,
                    stack_depth,
                };
                self.frames[fi].exception_handlers.push(handler);
            }

            Opcode::SETUP_CLEANUP => {
                let stack_depth = self.frames[fi].stack.len();
                let handler = ExceptionHandler {
                    instr_addr: arg as usize,
                    stack_depth,
                };
                self.frames[fi].exception_handlers.push(handler);
            }

            Opcode::POP_BLOCK => {
                // Restore stack to the depth before the handler was set up
                if let Some(handler) = self.frames[fi].exception_handlers.pop() {
                    self.frames[fi].stack.truncate(handler.stack_depth);
                }
            }

             Opcode::PUSH_EXC_INFO => {
                if arg != 1 {
                    let frame = &mut self.frames[fi];
                    frame
                        .active_exception_stack
                        .push(frame.active_exception.take());
                    if let Ok(exc) = frame.peek(0) {
                        frame.active_exception = Some(Box::new(exc));
                    }
                    // Track the handled exception for PEP 3134 implicit
                    // __context__ chaining. VM-global so calls made from within
                    // the handler see it too; the exception is now "handled"
                    // (no longer propagating). The value-stack depth lets
                    // `handle_exception`'s unwind drop entries whose handler
                    // was abandoned mid-body.
                    if let Ok(exc) = self.frames[fi].peek(0) {
                        let value_depth = self.frames[fi].stack.len() - 1;
                        if std::env::var("RPY_DEBUG_CTX").is_ok() {
                            eprintln!(
                                "PUSH_EXC: {} (stack now {})",
                                exc.borrow().repr(),
                                self.exc_context_stack.len() + 1
                            );
                        }
                        self.exc_context_stack.push((exc, value_depth));
                        self.propagating_exc = None;
                    }
                } else if let Ok(exc) = self.frames[fi].peek(0) {
                    self.frames[fi].active_exception = Some(Box::new(exc));
                }
            }

            Opcode::CLEAR_EXCEPTION_INFO => {
                // except*: drop the active exception so the following RERAISE
                // pops the UNMATCHED ExceptionGroup from the value stack
                // (RERAISE prefers active_exception, which still holds the
                // ORIGINAL exception — re-raising that would re-raise even a
                // fully-handled group).
                self.frames[fi].active_exception = None;
            }

            Opcode::POP_EXCEPT => {
                // Pop the exception object from the value stack.
                // In CPython this operates on a separate block stack for
                // exception info (type, value, traceback). Since RustPython
                // places the exception directly on the value stack, we pop
                // it here. The exception may already have been consumed by
                // STORE_NAME/STORE_FAST (handler with 'as e'), or it may
                // still be on the stack (handler without 'as e').
                self.frames[fi].stack.pop();
                // Handler finished — the handled exception is no longer the
                // active context for any later raise. (arg=1 marks the
                // finally-block counterpart of PUSH_EXC_INFO arg=1, which
                // never pushed onto the context stack in the first place.)
                if arg != 1 {
                    if std::env::var("RPY_DEBUG_CTX").is_ok() {
                        eprintln!("POP_EXC: (stack was {})", self.exc_context_stack.len());
                    }
                    self.exc_context_stack.pop();
                    // Restore the previous active_exception (exc_info stack
                    // semantics): after an inner handler ends, a bare `raise`
                    // re-raises the OUTER handler's exception. Deliberately
                    // skipped for arg=1 (finally): its RERAISE still needs
                    // the exception in active_exception.
                    if let Some(prev) = self.frames[fi].active_exception_stack.pop() {
                        self.frames[fi].active_exception = prev;
                    }
                    // Global exc_info cleanup: when the OUTERMOST handler for
                    // an exception finishes, sys.exc_info() must go back to
                    // (None, None, None). Without this the handled
                    // exception's traceback kept its origin frame_object (and
                    // through it, that frame's f_locals snapshot) alive
                    // indefinitely -- a real leak observed as weakrefs to
                    // handler-local objects never clearing.
                    if self.frames[fi].active_exception.is_none() {
                        let in_outer_handler = self
                            .frames
                            .iter()
                            .any(|f| f.active_exception.is_some());
                        if !in_outer_handler {
                            self.exc_type = None;
                            self.exc_value = None;
                        }
                    }
                }
            }

            Opcode::GET_AITER => {
                // async for: call __aiter__ on the top of stack
                let obj = self.frames[fi].peek(0)?;
                let aiter_method = obj
                    .borrow()
                    .get_attribute("__aiter__")
                    .map_err(|_| PyError::type_error("object does not support async iteration"))?;
                let result = self.call_function(aiter_method, vec![], vec![])?;
                let _ = self.frames[fi].pop();
                self.frames[fi].push(result);
            }

            Opcode::GET_ANEXT => {
                // async for: get __anext__ method from the async iterator
                let obj = self.frames[fi].peek(0)?;
                let anext_method = obj
                    .borrow()
                    .get_attribute("__anext__")
                    .map_err(|_| PyError::type_error("async iterator has no __anext__"))?;
                let _ = self.frames[fi].pop();
                self.frames[fi].push(anext_method);
            }

            Opcode::END_FOR => {
                // Pop the iterator/async-iterator after a for loop
                let _ = self.frames[fi].pop();
            }

            Opcode::BEFORE_ASYNC_WITH => {
                let mgr = self.frames[fi].pop()?;
                // Check __aexit__ first (mirroring SETUP_WITH's __exit__ check)
                let aexit = mgr.borrow().get_attribute("__aexit__").ok();
                if aexit.is_none() {
                    let has_enter = mgr.borrow().get_attribute("__enter__").is_ok();
                    let has_exit = mgr.borrow().get_attribute("__exit__").is_ok();
                    let supports_sync = has_enter && has_exit;
                    return Err(PyError::type_error(if supports_sync {
                        "object does not support the asynchronous context manager protocol (missed __aexit__ method) but it supports the context manager protocol. Did you mean to use 'with'?"
                    } else {
                        "object does not support the asynchronous context manager protocol (missed __aexit__ method)"
                    }));
                }
                let aenter_raw = mgr.borrow().get_attribute("__aenter__").ok();
                if let Some(aenter_raw) = aenter_raw {
                    let is_builtin = matches!(&*aenter_raw.borrow(), PyObject::BuiltinMethod { .. });
                    let bound = if is_builtin {
                        let b = aenter_raw.borrow();
                        match &*b {
                            PyObject::BuiltinMethod { name, func, .. } => {
                                PyObjectRef::imm(PyObject::BuiltinMethod {
                                    name: name.clone(),
                                    func: *func,
                                    self_obj: mgr.clone(),
                                })
                            }
                            _ => unreachable!(),
                        }
                    } else {
                        PyObjectRef::imm(PyObject::BoundMethod {
                            func: aenter_raw,
                            self_obj: mgr.clone(),
                        })
                    };
                    let result = self.call_function(bound, vec![], vec![])?;
                    self.frames[fi].push(mgr);
                    self.frames[fi].push(result);
                } else {
                    return Err(PyError::type_error(
                        "object does not support the asynchronous context manager protocol (missed __aenter__ method)",
                    ));
                }
            }

            Opcode::CHECK_EXC_MATCH => {
                let expected = self.frames[fi].pop()?;
                let exc = self.frames[fi].pop()?;
                let is_instance = matches!(&*exc.borrow(), PyObject::Instance { .. });
                let matched = if is_instance {
                    // A user-defined exception CLASS instance (`class
                    // MyError(Exception): ...`, `raise MyError(...)`) — used
                    // to fall straight to `None => false` below, since only
                    // the native `PyObject::Exception`/`ExceptionGroup`
                    // representations were recognized at all. This meant
                    // `except AnythingAtAll:` NEVER matched ANY user-defined
                    // exception class, no matter what it inherited from —
                    // confirmed via the simplest possible repro (`class
                    // MyError(Exception): pass; raise MyError("x")` not even
                    // caught by `except MyError:`, an EXACT match). Delegate
                    // to `isinstance()`, which is the real semantic `except`
                    // matching means, and which already correctly walks both
                    // custom-class mro (exact/ancestor custom matches) and
                    // `find_exception_base_name` (matches against whatever
                    // real builtin exception the class ultimately derives
                    // from, e.g. `except AttributeError:` catching a
                    // `class Foo(AttributeError): ...`).
                    crate::object::builtin_isinstance(&[exc.clone(), expected.clone()])?.truthy()
                } else {
                    let typ_name = match &*exc.borrow() {
                        PyObject::Exception { typ, .. } => Some(typ.clone()),
                        PyObject::ExceptionGroup { typ, .. } => Some(typ.clone()),
                        _ => None,
                    };
                    match typ_name {
                        Some(t) => exc_type_matches(&expected, &t)?,
                        None => false,
                    }
                };
                self.frames[fi].push(py_bool(matched));
            }

            Opcode::CHECK_EXC_MATCH_STAR => {
                // For except*: splits ExceptionGroup into matched/unmatched subgroups.
                // Pops 3 items (type, exc_dup from DUP_TOP, exc_orig from before DUP_TOP).
                // On match: pushes [unmatched_eg_or_empty_eg, matched_eg, True].
                // On no match: pushes [exc_orig, False].
                let expected = self.frames[fi].pop()?;
                let exc_dup = self.frames[fi].pop()?;
                let exc_orig = self.frames[fi].pop()?;

                // Read the type info from exc_dup while we still hold the borrow
                let is_eg = match &*exc_dup.borrow() {
                    PyObject::ExceptionGroup { .. } => true,
                    _ => false,
                };

                if is_eg {
                    // Read fully from the borrow so we can drop it
                    let (typ, args, matched, unmatched) = {
                        let eg = &*exc_dup.borrow();
                        let (typ, args, exceptions) = match eg {
                            PyObject::ExceptionGroup {
                                typ,
                                args,
                                exceptions,
                            } => (typ.clone(), args.clone(), exceptions.clone()),
                            _ => unreachable!(),
                        };
                        let mut matched = Vec::new();
                        let mut unmatched = Vec::new();
                        for child in &exceptions {
                            let child_name = match &*child.borrow() {
                                PyObject::Exception { typ, .. } => typ.clone(),
                                PyObject::ExceptionGroup { typ, .. } => typ.clone(),
                                _ => String::new(),
                            };
                            if exc_type_matches(&expected, &child_name)? {
                                matched.push(child.clone());
                            } else {
                                unmatched.push(child.clone());
                            }
                        }
                        (typ, args, matched, unmatched)
                    };

                    if !matched.is_empty() {
                        let matched_group = PyObjectRef::new(PyObject::ExceptionGroup {
                            typ: typ.clone(),
                            args: args.clone(),
                            exceptions: matched,
                        });
                        if !unmatched.is_empty() {
                            let unmatched_group = PyObjectRef::new(PyObject::ExceptionGroup {
                                typ: typ.clone(),
                                args: vec![py_str(&typ)],
                                exceptions: unmatched,
                            });
                            self.frames[fi].push(unmatched_group);
                        } else {
                            let empty_group = PyObjectRef::new(PyObject::ExceptionGroup {
                                typ: typ.clone(),
                                args: vec![py_str(&typ)],
                                exceptions: vec![],
                            });
                            self.frames[fi].push(empty_group);
                        }
                        self.frames[fi].push(matched_group);
                        self.frames[fi].push(py_bool(true));
                    } else {
                        // No matching children: restore original exception
                        self.frames[fi].push(exc_orig);
                        self.frames[fi].push(py_bool(false));
                    }
                } else {
                    // Not an ExceptionGroup — normal match check
                    let typ_name = match &*exc_dup.borrow() {
                        PyObject::Exception { typ, .. } => Some(typ.clone()),
                        _ => None,
                    };
                    let matched = match typ_name {
                        Some(t) => exc_type_matches(&expected, &t)?,
                        None => false,
                    };
                    if matched {
                        let empty_group = PyObjectRef::new(PyObject::ExceptionGroup {
                            typ: "ExceptionGroup".to_string(),
                            args: vec![py_str("")],
                            exceptions: vec![],
                        });
                        self.frames[fi].push(empty_group);
                        self.frames[fi].push(exc_dup);
                        self.frames[fi].push(py_bool(true));
                    } else {
                        self.frames[fi].push(exc_orig);
                        self.frames[fi].push(py_bool(false));
                    }
                }
            }

            Opcode::RERAISE => {
                // Prefer active_exception (set by PUSH_EXC_INFO) so that
                // POP_EXCEPT (which pops from the value stack) does not break
                // RERAISE in try/finally blocks.
                let reraise_exc = if let Some(exc) = self.frames[fi].active_exception.take() {
                    *exc
                } else if let Ok(exc) = self.frames[fi].pop() {
                    exc
                } else if let Some((exc, _)) = self.exc_context_stack.last().cloned() {
                    // Bare re-raise in a callee frame of an except handler
                    // (`def f(): raise`; `except E: f()`): the handler's
                    // active_exception lives in the CALLER's frame, but the
                    // VM-global context stack still holds it.
                    exc
                } else {
                    if std::env::var("RPY_DEBUG_RERAISE").is_ok() {
                        eprintln!(
                            "RERAISE FAIL: func={} file={} stack_len={}",
                            self.frames[fi].code.name,
                            self.frames[fi].code.filename,
                            self.frames[fi].stack.len()
                        );
                    }
                    return Err(PyError::runtime_error("No active exception to re-raise"));
                };
                // Check if it's an empty ExceptionGroup (all exceptions were handled)
                let is_empty_eg = match &*reraise_exc.borrow() {
                    PyObject::ExceptionGroup { exceptions, .. } => exceptions.is_empty(),
                    _ => false,
                };
                if !is_empty_eg {
                    if std::env::var("RPY_DEBUG_RERAISE").is_ok() {
                        eprintln!(
                            "RERAISE: kind={:?} repr={}",
                            std::mem::discriminant(&*reraise_exc.borrow()),
                            reraise_exc.borrow().repr()
                        );
                    }
                    return Err(PyError::Exception("re-raise".to_string(), reraise_exc));
                }
                // Empty group — all exceptions handled, silently continue
            }

            Opcode::RAISE_VARARGS => {
                let nargs = arg;
                match nargs {
                    0 => {
                        // Bare raise: re-raise the current exception. Must
                        // check `active_exception` FIRST, matching RERAISE
                        // just above — an `except E as exc:` clause's `as
                        // exc` binding (STORE_FAST) already consumes the
                        // value-stack copy of the exception, so a bare
                        // `raise` later in that same handler (the standard
                        // `except BaseException as exc: if exc is not
                        // value: raise` idiom — real code, CPython's own
                        // `contextlib._GeneratorContextManager.__exit__`)
                        // found nothing on the stack and incorrectly failed
                        // with "No active exception to re-raise" even
                        // though the real exception was still available via
                        // `active_exception` (set by PUSH_EXC_INFO exactly
                        // for this purpose, per its own doc comment).
                        let reraise_exc = if let Some(exc) = self.frames[fi].active_exception.take()
                        {
                            Some(*exc)
                        } else if let Some(exc) = self.frames[fi].stack.pop() {
                            Some(exc)
                        } else {
                            // Bare raise in a callee frame of an except
                            // handler — fall back to the VM-global context.
                            self.exc_context_stack.last().map(|(e, _)| e.clone())
                        };
                        match reraise_exc {
                            Some(exc) => {
                                return Err(PyError::Exception(format!("re-raise"), exc));
                            }
                            None => {
                                return Err(PyError::runtime_error(
                                    "No active exception to re-raise",
                                ))
                            }
                        }
                    }
                    1 => {
                        let exc = self.frames[fi].pop()?;
                        // If the raised value is already a native exception
                        // representation, use it directly — `PyObject::Str`
                        // deliberately NOT included here (see below): a bare
                        // string is never a valid thing to raise in Python 3.
                        let is_callable = !matches!(
                            &*exc.borrow(),
                            PyObject::Exception { .. }
                                | PyObject::ExceptionGroup { .. }
                                | PyObject::Instance { .. }
                        );
                        let exc = if is_callable {
                            let exc_clone = exc.clone();
                            match self.call_function(exc_clone, vec![], vec![]) {
                                Ok(instance) => instance,
                                // Propagate genuine constructor errors
                                // (`class MyError(Exception): def __init__
                                // (self): raise RuntimeError()` — CPython
                                // propagates the RuntimeError, it doesn't
                                // become "must derive from BaseException").
                                // Non-callables (e.g. a bare string) fail in
                                // call_function with their own TypeError,
                                // which is still a TypeError.
                                Err(e) => return Err(e),
                            }
                        } else {
                            exc
                        };
                        // Verify the raised value genuinely derives from
                        // BaseException. Real Python allows raising any
                        // BaseException instance/class, but a plain
                        // `class Foo: pass` instance (or a bare string —
                        // caught by excluding `PyObject::Str` from
                        // `is_callable` above, which routes it through
                        // `call_function` on a non-callable and fails there)
                        // must raise `TypeError`, not be silently treated as
                        // a legitimate, uncatchable-by-`except TypeError`
                        // exception. Real trigger: CPython's own
                        // `test_baseexception.py::
                        // test_raise_new_style_non_exception`/
                        // `test_raise_string` — both `raise SomeInstance`
                        // (no Exception ancestor) and `raise "spam"`
                        // previously propagated the raw value uncaught by
                        // ANY `except` clause instead of raising `TypeError`.
                        // `PyObject::Exception`/`ExceptionGroup` are this
                        // interpreter's own native representations, always
                        // trusted; only a plain `PyObject::Instance` needs
                        // its class hierarchy actually checked here.
                        if let PyObject::Instance { typ, .. } = &*exc.borrow() {
                            if crate::object::find_exception_base_name(typ).is_none() {
                                return Err(PyError::type_error(
                                    "exceptions must derive from BaseException",
                                ));
                            }
                        }
                        let msg = match &*exc.borrow() {
                            PyObject::Str(s) => s.to_string(),
                            PyObject::Exception { args, .. } => {
                                if !args.is_empty() {
                                    args[0].str()
                                } else {
                                    "".to_string()
                                }
                            }
                            PyObject::ExceptionGroup { args, .. } => {
                                if !args.is_empty() {
                                    args[0].str()
                                } else {
                                    "".to_string()
                                }
                            }
                            PyObject::Instance { dict, .. } => {
                                // Extract error message from the instance
                                // Python stores exception args in self.args tuple
                                let args = dict.get_str("args");
                                if let Some(a) = args {
                                    let b = a.borrow();
                                    if let PyObject::Tuple(t) = &*b {
                                        if !t.is_empty() {
                                            t[0].str()
                                        } else {
                                            exc.repr()
                                        }
                                    } else {
                                        exc.repr()
                                    }
                                } else {
                                    // Fallback: repr of the exception object
                                    exc.repr()
                                }
                            }
                            _ => {
                                return Err(PyError::type_error(
                                    "exceptions must be str or Exception instances",
                                ))
                            }
                        };
                        // raise StopIteration → PyError::StopIteration (needed by for_iter/next protocol)
                        if msg.is_empty() {
                            let exc_borrowed = exc.borrow();
                            let is_stop = match &*exc_borrowed {
                                PyObject::Exception { ref typ, .. } if typ == "StopIteration" => {
                                    true
                                }
                                PyObject::Type { name, .. } if name == "StopIteration" => true,
                                _ => false,
                            };
                            if is_stop {
                                return Err(PyError::StopIteration);
                            }
                        }
                        // Store exc_info before returning error. `exc_type`
                        // must be the exception's real CLASS object (what
                        // real `sys.exc_info()[0]` is, and what makes
                        // `issubclass(exc_info()[0], SomeError)` — the
                        // pattern `unittest`'s own `TestResult`/`_Outcome`
                        // use to classify failures vs. errors — valid to
                        // call at all) — NOT the exception instance itself.
                        // This used to just be `exc.clone()` (the instance)
                        // for BOTH `exc_type` and `exc_value`, so any code
                        // calling `issubclass()` on it crashed with
                        // `TypeError: issubclass() arg 1 must be a class`
                        // the moment a real test failure/error tried to get
                        // reported through `unittest.main()`.
                        self.exc_type = Some(self.exception_class_of(&exc));
                        self.exc_value = Some(exc.clone());
                        self.exc_traceback = Some(py_none());
                        if std::env::var("RPY_DEBUG_EXCINFO").is_ok() {
                            eprintln!(
                                "RAISE set exc_type={} exc_value={}",
                                self.exc_type.as_ref().unwrap().repr(),
                                self.exc_value.as_ref().unwrap().repr()
                            );
                        }
                        self.capture_exception_context(&exc);
                        return Err(PyError::Exception(msg, exc));
                    }
                    2 => {
                        let cause = self.frames[fi].pop()?;
                        let exc = self.frames[fi].pop()?;
                        // Same class-instantiation as the nargs=1 arm:
                        // `raise ValueError from None` puts the exception
                        // *class* on the stack, which must be called to
                        // produce the instance before we can attach a cause.
                        // `cause` is also a class in `raise X from SomeClass`
                        // and must be instantiated the same way (CPython calls
                        // `SomeClass()` here — a `__new__` returning a
                        // non-BaseException triggers the type-construct
                        // validation added in `call_function`).
                        let exc = {
                            let is_callable = !matches!(
                                &*exc.borrow(),
                                PyObject::Exception { .. }
                                    | PyObject::ExceptionGroup { .. }
                                    | PyObject::Instance { .. }
                            );
                            if is_callable {
                                let exc_clone = exc.clone();
                                match self.call_function(exc_clone, vec![], vec![]) {
                                    Ok(instance) => instance,
                                    Err(e) => return Err(e),
                                }
                            } else {
                                exc
                            }
                        };
                        let cause_is_none = matches!(&*cause.borrow(), PyObject::None);
                        // Validate the cause BEFORE instantiating: `raise X
                        // from 5` must yield "exception causes must derive
                        // from BaseException" (a plain non-exception value),
                        // not "'int' object is not callable" from the
                        // instantiation attempt below.
                        if !cause_is_none {
                            let cause_kind = match &*cause.borrow() {
                                PyObject::Exception { .. } | PyObject::ExceptionGroup { .. } => {
                                    "exc"
                                }
                                PyObject::Instance { typ, .. } => {
                                    if crate::object::find_exception_base_name(typ).is_some() {
                                        "exc"
                                    } else {
                                        "bad"
                                    }
                                }
                                PyObject::Type { .. } => "class",
                                PyObject::BuiltinFunction { name, .. } => {
                                    // Builtin exception classes (KeyError,
                                    // ValueError, ...) are BuiltinFunctions.
                                    if crate::vm::is_exception_subclass(name, "BaseException") {
                                        "class"
                                    } else {
                                        "bad"
                                    }
                                }
                                _ => "bad",
                            };
                            if cause_kind == "bad" {
                                return Err(PyError::type_error(
                                    "exception causes must derive from BaseException",
                                ));
                            }
                        }
                        let cause = if cause_is_none {
                            cause
                        } else {
                            let is_callable = !matches!(
                                &*cause.borrow(),
                                PyObject::Exception { .. }
                                    | PyObject::ExceptionGroup { .. }
                                    | PyObject::Instance { .. }
                            );
                            if is_callable {
                                let cause_clone = cause.clone();
                                match self.call_function(cause_clone, vec![], vec![]) {
                                    Ok(instance) => instance,
                                    Err(e) => return Err(e),
                                }
                            } else {
                                cause
                            }
                        };
                        // Validate the cause is None or a BaseException
                        // (CPython: "exception causes must derive from
                        // BaseException" — `raise X from 5` must not reach
                        // the __cause__ assignment with a raw int).
                        if !cause_is_none {
                            let is_exc = match &*cause.borrow() {
                                PyObject::Exception { .. } | PyObject::ExceptionGroup { .. } => {
                                    true
                                }
                                PyObject::Instance { typ, .. } => {
                                    crate::object::find_exception_base_name(typ).is_some()
                                }
                                _ => false,
                            };
                            if !is_exc {
                                return Err(PyError::type_error(
                                    "exception causes must derive from BaseException",
                                ));
                            }
                        }
                        let exc_msg = match &*exc.borrow() {
                            PyObject::Exception { args, .. } => {
                                if !args.is_empty() {
                                    args[0].str()
                                } else {
                                    exc.str()
                                }
                            }
                            _ => exc.str(),
                        };
                        let cause_msg = match &*cause.borrow() {
                            PyObject::Exception { args, .. } => {
                                if !args.is_empty() {
                                    args[0].str()
                                } else {
                                    cause.str()
                                }
                            }
                            _ if cause_is_none => String::new(),
                            _ => cause.str(),
                        };
                        // Set __cause__ on the exception object. The native
                        // `PyObject::Exception` representation has a
                        // dedicated `cause` field; a user-defined exception
                        // class (`class MyError(Exception): ...` — a plain
                        // `PyObject::Instance`, the overwhelming majority of
                        // real exception classes) has no such field, so
                        // `raise X from Y` previously did NOTHING for it —
                        // `X.__cause__` stayed `None` regardless, silently
                        // dropping the explicit cause entirely. `Instance`'s
                        // own `set_attribute` already supports arbitrary
                        // attributes with no restriction, so just storing
                        // `__cause__` there directly works, matching how
                        // reading it back (`get_attribute_impl`'s Instance
                        // arm) already checks the instance's own dict first.
                        match &mut *exc.borrow_mut() {
                            PyObject::Exception {
                                cause: ref mut cause_field,
                                suppress_context: ref mut suppress_field,
                                ..
                            } => {
                                // `from None` suppresses implicit chaining
                                // (PEP 3134): cause stays None and
                                // __suppress_context__ becomes True.
                                let is_none = matches!(&*cause.borrow(), PyObject::None);
                                *cause_field = if cause_is_none {
                                    None
                                } else {
                                    Some(cause.clone())
                                };
                                *suppress_field = cause_is_none;
                            }
                            PyObject::Instance { dict, .. } => {
                                dict.insert_str("__cause__", cause.clone());
                                dict.insert_str("__suppress_context__", py_bool(cause_is_none));
                            }
                            _ => {}
                        }
                        let err_msg = if cause_msg.is_empty() {
                            exc_msg
                        } else {
                            format!("{} (caused by: {})", exc_msg, cause_msg)
                        };
                        self.capture_exception_context(&exc);
                        return Err(PyError::Exception(err_msg, exc));
                    }
                    _ => return Err(PyError::runtime_error("invalid RAISE_VARARGS count")),
                }
            }

            Opcode::IMPORT_NAME => {
                let name_idx = arg as usize;
                let name = crate::interner::lookup(self.frames[fi].code.names[name_idx]);
                // Pop level (int, TOS) and fromlist (TOS1)
                let level_val = self.frames[fi].pop()?;
                let _fromlist = self.frames[fi].pop()?;
                // Resolve relative imports: if level > 0, use __package__ from frame globals
                let resolved = {
                    let level = {
                        let obj = level_val.borrow();
                        match &*obj {
                            PyObject::Int(i) => i.to_i64().unwrap_or(0) as usize,
                            _ => 0,
                        }
                    };
                    if level > 0 {
                        let pkg = self.frames[fi]
                            .globals
                            .borrow()
                            .get(&interner::intern("__package__"))
                            .cloned()
                            .and_then(|p| {
                                let p = p.borrow();
                                if let PyObject::Str(s) = &*p {
                                    Some(s.to_string())
                                } else {
                                    None
                                }
                            });
                        // level=1 (`from . import x`) resolves relative to
                        // __package__ itself; each additional dot
                        // (level=2 → `from .. import x`, etc.) goes up one
                        // more enclosing package, stripping one more
                        // trailing component. This was previously ignored
                        // entirely — `from .. import x` resolved identically
                        // to `from . import x`, silently importing from the
                        // wrong (child, not parent) package whenever a
                        // module used a multi-dot relative import.
                        let pkg = pkg.map(|p| {
                            let mut segs: Vec<&str> = p.split('.').collect();
                            let strip = level.saturating_sub(1);
                            if strip >= segs.len() {
                                segs.clear();
                            } else {
                                segs.truncate(segs.len() - strip);
                            }
                            segs.join(".")
                        });
                        let resolved_name = match pkg {
                            Some(p) if !p.is_empty() => {
                                if name.is_empty() {
                                    p
                                } else {
                                    format!("{}.{}", p, name)
                                }
                            }
                            // Fallback: use __name__ up to last dot as package
                            _ => {
                                let n = self.frames[fi]
                                    .globals
                                    .borrow()
                                    .get(&interner::intern("__name__"))
                                    .cloned()
                                    .and_then(|n| {
                                        let n = n.borrow();
                                        if let PyObject::Str(s) = &*n {
                                            Some(s.to_string())
                                        } else {
                                            None
                                        }
                                    })
                                    .unwrap_or_default();
                                if let Some(dot) = n.rfind('.') {
                                    let base = &n[..dot];
                                    if name.is_empty() {
                                        base.to_string()
                                    } else {
                                        format!("{}.{}", base, name)
                                    }
                                } else {
                                    name.clone()
                                }
                            }
                        };
                        resolved_name
                    } else {
                        name.clone()
                    }
                };
                if std::env::var("RPY_DEBUG_IMPORT").is_ok() {
                    eprintln!(
                        "IMPORT_NAME: resolved={} cached={}",
                        resolved,
                        self.modules.contains_key(&resolved)
                    );
                }
                if let Some(module) = self.import_cached_or_fresh(&resolved) {
                    // For 'import a.b.c' where fromlist is empty (regular import, not 'from a.b import X'),
                    // push the top-level module so STORE_NAME stores the package, not the submodule
                    let is_from_import = {
                        let obj = _fromlist.borrow();
                        matches!(&*obj, PyObject::Tuple(items) if !items.is_empty())
                    };
                    if resolved.contains('.') && !is_from_import {
                        // Set sub-module as attribute on parent module (e.g. logging.config = <module>)
                        if let Some(dot_pos) = resolved.rfind('.') {
                            let parent_name = &resolved[..dot_pos];
                            let child_name = &resolved[dot_pos + 1..];
                            if let Some(parent_mod) = self.modules.get(parent_name) {
                                let _ = parent_mod
                                    .borrow_mut()
                                    .set_attribute(child_name, module.clone());
                            }
                        }
                        if let Some(top) = resolved.split('.').next() {
                            if let Some(top_mod) = self.modules.get(top) {
                                self.frames[fi].push(top_mod.clone());
                            } else {
                                self.frames[fi].push(module.clone());
                            }
                        } else {
                            self.frames[fi].push(module.clone());
                        }
                    } else {
                        self.frames[fi].push(module.clone());
                    }
                } else {
                    match self.import_module_from_file(&resolved) {
                        Ok(module) => {
                            self.modules.insert(resolved.clone(), module.clone());
                            // Register in sys.modules (safe: module fully loaded)
                            if let Some(sys_mod) = self.modules.get("sys") {
                                if let PyObject::Module { dict, .. } = &*sys_mod.borrow() {
                                    if let Some(md) = dict.get_str("modules").cloned() {
                                        match &md {
                                            PyObjectRef::Mut(rc) => {
                                                if let Ok(mut guard) = rc.try_borrow_mut() {
                                                    if let PyObject::Dict(ref mut d) = &mut *guard {
                                                        d.set(py_str(&resolved), module.clone())
                                                            .ok();
                                                    }
                                                }
                                            }
                                            _ => {}
                                        }
                                    }
                                }
                            }
                            self.frames[fi].push(module);
                            // For 'import a.b.c' where fromlist is empty,
                            // push top-level module instead of deepest module
                            let is_from_import = {
                                let obj = _fromlist.borrow();
                                matches!(&*obj, PyObject::Tuple(items) if !items.is_empty())
                            };
                            if resolved.contains('.') && !is_from_import {
                                if let Some(top) = resolved.split('.').next() {
                                    if let Some(top_mod) = self.modules.get(top) {
                                        let _ = self.frames[fi].pop();
                                        self.frames[fi].push(top_mod.clone());
                                    }
                                }
                            }
                        }
                        Err(e) => return Err(e),
                    }
                }
            }

            Opcode::IMPORT_FROM => {
                let name_idx = arg as usize;
                let name = crate::interner::lookup(self.frames[fi].code.names[name_idx]);
                let module = self.frames[fi].peek(0)?;
                // Handle 'from module import *' — when the imported name is '*',
                // iterate over the module's dict and store all names in current scope
                if name == "*" {
                    let module_borrowed = module.borrow();
                    if let PyObject::Module { dict, .. } = &*module_borrowed {
                        // Use __all__ if present, otherwise all non-underscore names
                        let names_to_import: Vec<String> =
                            if let Some(all_val) = dict.get_str("__all__") {
                                let all_borrowed = all_val.borrow();
                                match &*all_borrowed {
                                    PyObject::Tuple(items) | PyObject::List(items) => items
                                        .iter()
                                        .filter_map(|n| {
                                            if let PyObject::Str(s) = &*n.borrow() {
                                                Some(s.to_string())
                                            } else {
                                                None
                                            }
                                        })
                                        .collect(),
                                    _ => dict
                                        .keys()
                                        .map(|k| interner::lookup_str(*k))
                                        .filter(|k| !k.starts_with('_'))
                                        .map(|k| k.to_string())
                                        .collect(),
                                }
                            } else {
                                dict.keys()
                                    .map(|k| interner::lookup_str(*k))
                                    .filter(|k| !k.starts_with('_'))
                                    .map(|k| k.to_string())
                                    .collect()
                            };
                        // Collect name-value pairs before dropping borrow
                        let imports: Vec<(String, PyObjectRef)> = names_to_import
                            .iter()
                            .filter_map(|name| {
                                dict.get_str(&name).map(|val| (name.clone(), val.clone()))
                            })
                            .collect();
                        drop(module_borrowed);
                        let live_module = self.frames[fi].live_module.clone();
                        for (import_name, val) in &imports {
                            if let Some(order) = self.frames[fi].name_order.clone() {
                                let mut order = order.borrow_mut();
                                if !order.contains(import_name) {
                                    order.push(import_name.clone());
                                }
                            }
                            if let Some(lm) = &live_module {
                                if let PyObject::Module { dict, .. } = &mut *lm.borrow_mut() {
                                    dict.insert_str(import_name, val.clone());
                                }
                            }
                            self.frames[fi]
                                .globals
                                .borrow_mut()
                                .insert(interner::intern(&import_name), val.clone());
                        }
                        // Push placeholder module result (the loop above already pushed values)
                        // The POP_TOP after IMPORT_FROM loop will clean up
                        self.frames[fi].push(py_none());
                        return Ok(None);
                    }
                }
                // Check if name is in module's dict first (without holding borrow)
                let found = {
                    let obj = module.borrow();
                    match &*obj {
                        PyObject::Module { dict, .. } => dict.get_str(&name).cloned(),
                        _ => return Err(PyError::runtime_error("IMPORT_FROM on non-module")),
                    }
                };
                // Get module name for submodule import (clone to avoid borrow conflicts)
                let module_name = {
                    let obj = module.borrow();
                    match &*obj {
                        PyObject::Module { name: mn, .. } => mn.clone(),
                        _ => return Err(PyError::runtime_error("IMPORT_FROM on non-module")),
                    }
                };
                // Circular-import fallback: if this module is STILL mid-execution
                // further down the call stack (e.g. its __init__.py does
                // `import package.submodule` as its last statement, and that
                // submodule does `from . import name_defined_earlier`), the
                // module object's own dict is only populated once the whole
                // body finishes — it's a snapshot copy, not a live view of the
                // executing frame's globals. Check ancestor frames' actual
                // live globals for the name before giving up.
                let found_direct = found.is_some();
                let found = found.or_else(|| {
                    self.frames.iter().find_map(|f| {
                        let g = f.globals.borrow();
                        if g.get(&interner::intern("__name__"))
                            .map(|n| n.str())
                            .as_deref()
                            == Some(module_name.as_str())
                        {
                            g.get(&interner::intern(&name)).cloned()
                        } else {
                            None
                        }
                    })
                });
                if std::env::var("RPY_DEBUG_IMPORT").is_ok() {
                    eprintln!(
                        "IMPORT_FROM: name={} module={} found_direct={} found_after_ancestor={}",
                        name,
                        module_name,
                        found_direct,
                        found.is_some()
                    );
                }
                if let Some(val) = found {
                    self.frames[fi].push(val);
                } else {
                    // Try importing as sub-module (for dotted names like os.path)
                    let submodule_name = format!("{}.{}", module_name, name);
                    if std::env::var("RPY_DEBUG_IMPORT").is_ok() {
                        eprintln!(
                            "IMPORT_FROM fallback: submodule_name={} already_cached={}",
                            submodule_name,
                            self.modules.contains_key(&submodule_name)
                        );
                    }
                    if submodule_name.contains('.') {
                        match self.import_module_from_file(&submodule_name) {
                            Ok(submod) => {
                                self.modules.insert(submodule_name.clone(), submod.clone());
                                if let PyObject::Module { dict, .. } = &mut *module.borrow_mut() {
                                    dict.insert_str(&name, submod.clone());
                                }
                                self.frames[fi].push(submod);
                            }
                            Err(e) => {
                                // RPY_DEBUG_IMPORT=1 prints the real underlying
                                // error before it potentially gets flattened
                                // into the generic message below (kept
                                // permanently per user request, 2026-07-19).
                                if std::env::var("RPY_DEBUG_IMPORT").is_ok() {
                                    eprintln!(
                                        "IMPORT_FROM_FAIL: name={} module={} err={}",
                                        name, module_name, e
                                    );
                                }
                                // Only "the submodule doesn't exist" collapses
                                // to CPython's generic "cannot import name"
                                // message here. Any other error (e.g. the
                                // submodule's own body raising a real
                                // exception) must propagate as that real
                                // exception — previously this branch always
                                // discarded it in favor of the generic
                                // ImportError, so a genuine bug inside an
                                // imported submodule was indistinguishable
                                // from the name simply not existing.
                                if matches!(e, PyError::ImportError(_)) {
                                    return Err(PyError::ImportError(format!(
                                        "cannot import name '{}' from '{}'",
                                        name, module_name
                                    )));
                                }
                                return Err(e);
                            }
                        }
                    } else {
                        return Err(PyError::ImportError(format!(
                            "cannot import name '{}' from '{}'",
                            name, module_name
                        )));
                    }
                }
            }

            Opcode::IMPORT_STAR => {
                // `from x import *` — copy every public (non-underscore,
                // subject to `__all__`) name from the module into the current
                // namespace (test_pkg::test_2). The module is on the stack.
                let module = self.frames[fi].pop()?;
                let names: Vec<String> = {
                    let obj = module.borrow();
                    match &*obj {
                        PyObject::Module { dict, .. } => {
                            if let Some(all) = dict.get_str("__all__").cloned() {
                                match &*all.borrow() {
                                    PyObject::List(items) | PyObject::Tuple(items) => {
                                        items.iter().map(|i| i.str()).collect()
                                    }
                                    _ => Vec::new(),
                                }
                            } else {
                                let mut v: Vec<String> = dict
                                    .iter()
                                    .map(|(k, _)| interner::lookup_str(*k))
                                    .filter(|k| !k.starts_with('_'))
                                    .map(|k| k.to_string())
                                    .collect();
                                v.sort();
                                v
                            }
                        }
                        _ => return Err(PyError::runtime_error("IMPORT_STAR on non-module")),
                    }
                };
                let mut g = self.frames[fi].globals.borrow_mut();
                for n in &names {
                    let obj = module.borrow();
                    if let PyObject::Module { dict, .. } = &*obj {
                        if let Some(v) = dict.get(&interner::intern(n)).cloned() {
                            g.insert(interner::intern(n), v);
                        }
                    }
                }
                drop(g);
                // Also mirror into frame.locals so subsequent LOAD_NAME in
                // the same scope sees the names (exec with separate locals).
                let mut locals = self.frames[fi].locals.clone();
                for n in &names {
                    let obj = module.borrow();
                    if let PyObject::Module { dict, .. } = &*obj {
                        if let Some(v) = dict.get(&interner::intern(n)).cloned() {
                            locals.insert(interner::intern(n), v);
                        }
                    }
                }
            }

            Opcode::LOAD_BUILD_CLASS => {
                self.frames[fi].push(PyObjectRef::imm(PyObject::BuildClass));
            }

            Opcode::LOAD_CLOSURE => {
                let idx = arg as usize;
                let cell = {
                    let f = &self.frames[self.frames.len() - 1];
                    if idx < f.code.cellvars.len() {
                        let name = &f.code.cellvars[idx];
                        if let Some(var_idx) = f
                            .code
                            .varnames
                            .iter()
                            .position(|&n| crate::interner::intern_eq(n, name))
                        {
                            if let Some(val) = f.fast_locals.get(var_idx).and_then(|v| v.clone()) {
                                val
                            } else {
                                PyObjectRef::new(PyObject::Cell { value: None })
                            }
                        } else {
                            PyObjectRef::new(PyObject::Cell { value: None })
                        }
                    } else {
                        let fv_idx = idx - f.code.cellvars.len();
                        if let Some(cell) = f.closure.get(fv_idx).cloned() {
                            cell
                        } else {
                            PyObjectRef::new(PyObject::Cell { value: None })
                        }
                    }
                };
                self.frames[fi].push(cell);
            }

            Opcode::FORMAT_SIMPLE => {
                let val = self.frames[fi].pop()?;
                self.frames[fi].push(py_str(&val.str()));
            }

            Opcode::FORMAT_WITH_SPEC => {
                let spec = self.frames[fi].pop()?;
                let val = self.frames[fi].pop()?;
                let spec_str = spec.str();
                self.frames[fi].push(py_str(&format_with_spec(&val, &spec_str)?));
            }

            Opcode::CONVERT_VALUE => {
                let conversion = arg;
                let val = self.frames[fi].pop()?;
                let result = match conversion {
                    0 => py_str(&val.str()),
                    1 => py_str(&val.repr()),
                    2 => py_str(&val.str()),
                    3 => {
                        // !a (ascii) conversion: repr() with non-ASCII escaped
                        let s = val.repr();
                        let escaped: String = s
                            .chars()
                            .flat_map(|c| {
                                if c.is_ascii() {
                                    c.to_string().chars().collect::<Vec<_>>()
                                } else {
                                    c.escape_unicode().collect::<Vec<_>>()
                                }
                            })
                            .collect();
                        py_str(&escaped)
                    }
                    _ => return Err(PyError::runtime_error("unknown conversion type")),
                };
                self.frames[fi].push(result);
            }

            Opcode::LOAD_LOCALS => {
                self.frames[fi].push(py_dict());
            }

            Opcode::SETUP_ANNOTATIONS => {
                let ann_id = crate::interner::intern("__annotations__");
                let has = {
                    let frame = &self.frames[fi];
                    frame.locals.contains_key(ann_id)
                        || frame.globals.borrow().contains_key(&ann_id)
                        || frame
                            .module_globals
                            .as_ref()
                            .map_or(false, |mg| mg.borrow().contains_key(&ann_id))
                };
                if !has {
                    let ann_dict = crate::object::py_dict();
                    // Class bodies store names in globals (the namespace dict),
                    // while also keeping locals for fast access; insert into both
                    // so LOAD_NAME finds it via either path.
                    self.frames[fi].locals.insert(ann_id, ann_dict.clone());
                    self.frames[fi].globals.borrow_mut().insert(ann_id, ann_dict);
                }
            }

            Opcode::POP_ITER => {
                self.frames[fi].pop()?;
            }

            Opcode::SETUP_WITH => {
                // Look up __enter__ and call it, keeping manager on stack
                let mgr = self.frames[fi].peek(0)?;
                let exit_method = mgr.borrow().get_attribute("__exit__").ok();
                if exit_method.is_none() {
                    let has_aenter = mgr
                        .borrow()
                        .get_attribute("__aenter__")
                        .is_ok_and(|v| !matches!(&*v.borrow(), PyObject::None));
                    return Err(PyError::type_error(if has_aenter {
                        "object does not support the context manager protocol (missed __exit__ method) \
                         but it supports the asynchronous context manager protocol. \
                         Did you mean to use 'async with'?"
                    } else {
                        "object does not support the context manager protocol (missed __exit__ method)"
                    }));
                }
                let enter_raw = mgr.borrow().get_attribute("__enter__").ok();
                if let Some(enter_raw) = enter_raw {
                    let is_builtin = matches!(&*enter_raw.borrow(), PyObject::BuiltinMethod { .. });
                    let enter = if is_builtin {
                        let b = enter_raw.borrow();
                        match &*b {
                            PyObject::BuiltinMethod { name, func, .. } => {
                                PyObjectRef::imm(PyObject::BuiltinMethod {
                                    name: name.clone(),
                                    func: *func,
                                    self_obj: mgr.clone(),
                                })
                            }
                            _ => unreachable!(),
                        }
                    } else {
                        PyObjectRef::imm(PyObject::BoundMethod {
                            func: enter_raw,
                            self_obj: mgr.clone(),
                        })
                    };
                    let result = self.call_function(enter, vec![], vec![])?;
                    self.frames[fi].push(result);
                } else {
                    return Err(PyError::type_error(
                        "object does not support the context manager protocol (missed __enter__ method)",
                    ));
                }
            }

            Opcode::WITH_EXIT => {
                // Stack: [..., exception_obj, manager]
                // Call manager.__exit__(exc_type, exc_val, traceback) — exc_type
                // and exc_val must be the real class object and exception
                // instance (not a bare type-name string / the first ctor arg),
                // since __exit__ implementations commonly do isinstance(value,
                // ...), re-raise `value`, or read value.args/__traceback__.
                let mgr = self.frames[fi].pop()?;
                let (typ_obj, val) = {
                    let exc = self.frames[fi].peek(0)?;
                    let exc_borrowed = exc.borrow();
                    match &*exc_borrowed {
                        PyObject::Exception { typ, .. } => {
                            let typ_obj = self.frames[fi]
                                .builtins
                                .get(&interner::intern(&typ))
                                .cloned()
                                .unwrap_or_else(|| py_str(typ));
                            (typ_obj, exc.clone())
                        }
                        // A user-defined exception CLASS instance (`class
                        // MyError(Exception): ...`, `raise MyError(...)`)
                        // only ever matched the native `PyObject::Exception`
                        // arm above, silently falling through to `(None,
                        // None)` here — meaning `__exit__(exc_type,
                        // exc_value, tb)` was ALWAYS called as if no
                        // exception had occurred whenever the `with` body
                        // raised a custom exception class, not just the
                        // handful of natively-represented ones. Same root
                        // gap as `CHECK_EXC_MATCH`/`isinstance`/`issubclass`
                        // (already fixed elsewhere this session for their
                        // own call sites) — this is the `with`-statement's
                        // own, previously-unfixed instance of it. Real
                        // trigger: `unittest`'s own `assertRaises`, whose
                        // `_AssertRaisesBaseContext.__exit__` checks `if
                        // exc_type is None: <fail: "X not raised">` — ANY
                        // `assertRaises(CustomExceptionClass, ...)` call
                        // spuriously reported the exception as never having
                        // been raised at all, even though it genuinely was.
                        PyObject::Instance { typ, .. } => (typ.clone(), exc.clone()),
                        _ => (py_none(), py_none()),
                    }
                };
                let exit_raw = mgr
                    .borrow()
                    .get_attribute("__exit__")
                    .map_err(|_| PyError::attribute_error("context manager has no __exit__"))?;
                // A method found directly on a native type (e.g. `lock.
                // __exit__`, `Lock`'s attribute lookup in `attrs.rs`) comes
                // back as a `BuiltinMethod` with a PLACEHOLDER `self_obj`
                // (see `NATIVE_VALUE_CTOR_KEY`'s doc comment) — wrapping
                // THAT placeholder-carrying value inside another
                // `BoundMethod{self_obj: mgr}` (the old code here) never
                // actually rebinds it to the real manager: `mgr` never
                // reaches the native implementation, so e.g. `Lock.__exit__`
                // silently no-ops instead of clearing the lock flag, and
                // ANY subsequent `with lock:` on the SAME lock hangs forever
                // spinning on a flag that can never become false again.
                // Mirrors the exact unwrap-and-rebuild-with-the-real-
                // self_obj pattern `SETUP_WITH`'s own `__enter__` handling
                // (just above) already uses — `WITH_EXIT` needs the
                // identical treatment, not a second, ineffective wrapping.
                let is_builtin = matches!(&*exit_raw.borrow(), PyObject::BuiltinMethod { .. });
                let bound = if is_builtin {
                    let b = exit_raw.borrow();
                    match &*b {
                        PyObject::BuiltinMethod { name, func, .. } => {
                            PyObjectRef::imm(PyObject::BuiltinMethod {
                                name: name.clone(),
                                func: *func,
                                self_obj: mgr.clone(),
                            })
                        }
                        _ => unreachable!(),
                    }
                } else {
                    PyObjectRef::imm(PyObject::BoundMethod {
                        func: exit_raw,
                        self_obj: mgr,
                    })
                };
                // Pass the exception's real __traceback__ (not py_none) —
                // `__exit__(type, value, tb)` implementations and
                // test_with's MockContextManager assert tb is non-None after
                // an exception (exit_args[2] is not None).
                let tb_arg = val
                    .borrow()
                    .get_attribute("__traceback__")
                    .unwrap_or_else(|_| py_none());
                let result = self.call_function(bound, vec![typ_obj, val, tb_arg], vec![])?;
                self.frames[fi].push(result);
            }

            Opcode::YIELD_VALUE => {
                let val = self.frames[fi].pop()?;
                // Don't push a placeholder; the Generator/Coroutine send method
                // will push the actual sent value (or None for __next__) onto
                // the frame stack, making it available for the next instruction.
                return Ok(Some(val));
            }

            Opcode::RETURN_GENERATOR => {
                // Create a Generator or Coroutine wrapping current frame
                let is_coroutine = self.frames[fi].code.flags & 0x100 != 0;
                let frame = self.frames[fi].clone();
                if is_coroutine {
                    let gen = PyObjectRef::new(PyObject::Coroutine {
                        frame: std::cell::RefCell::new(Some(Box::new(frame))),
                    });
                    return Ok(Some(gen));
                } else {
                    let gen = PyObjectRef::new(PyObject::Generator {
                        frame: std::cell::RefCell::new(Some(Box::new(frame))),
                    });
                    return Ok(Some(gen));
                }
            }

            Opcode::GET_AWAITABLE => {
                // Call __await__ on the object to get an iterator
                let obj = self.frames[fi].pop()?;
                let await_method = obj
                    .borrow()
                    .get_attribute("__await__")
                    .map_err(|_| PyError::type_error("object does not support __await__"))?;
                // `get_attribute` on a `Coroutine`/`Generator` returns
                // `__await__` as a `BuiltinMethod` with a `None` PLACEHOLDER
                // `self_obj` (it has no access to the enclosing `PyObjectRef`
                // from inside `impl PyObject`'s `&self`-only method) — its
                // closure body (`|args| Ok(args[0].clone())`, i.e. "return
                // self") then returned that placeholder `None` instead of
                // the real coroutine, so EVERY `await some_async_fn()` where
                // `some_async_fn` itself awaits something else (i.e. any
                // nested async call — confirmed via the simplest possible
                // repro, `async def foo(): return 1` / `async def main():
                // return await foo()`) pushed `None` as the "awaitable
                // iterator" instead of the coroutine, which the immediately
                // following `SEND` then rejected with `TypeError: SEND on
                // non-generator/coroutine/instance` — a fundamental,
                // previously-undetected break in the single most common
                // async/await pattern there is. Rebind `self_obj` to the
                // REAL object here before calling, exactly like the `SEND`
                // opcode handler already does for `send`/`throw` just below
                // (the established fix for this exact class of gap).
                let await_method = match &*await_method.borrow() {
                    PyObject::BuiltinMethod { name, func, .. } => {
                        PyObjectRef::imm(PyObject::BuiltinMethod {
                            name: name.clone(),
                            func: *func,
                            self_obj: obj.clone(),
                        })
                    }
                    _ => await_method.clone(),
                };
                let result = self.call_function(await_method, vec![], vec![])?;
                self.frames[fi].push(result);
            }

            Opcode::SEND => {
                // Send value into generator/coroutine/iterator: pop value, peek iterator
                let val = self.frames[fi].pop()?;
                let iter_val = self.frames[fi].peek(0)?;
                let result = {
                    // Try to find a send method on the iterator
                    let is_gen = matches!(&*iter_val.borrow(), PyObject::Generator { .. });
                    let is_coro = matches!(&*iter_val.borrow(), PyObject::Coroutine { .. });
                    if is_gen || is_coro {
                        let method_name = "send";
                        match iter_val.borrow().get_attribute(method_name) {
                            Ok(send_method) => {
                                let bound = match &*send_method.borrow() {
                                    PyObject::BuiltinMethod { func, .. } => {
                                        PyObjectRef::imm(PyObject::BuiltinMethod {
                                            name: "send".to_string(),
                                            func: *func,
                                            self_obj: iter_val.clone(),
                                        })
                                    }
                                    _ => {
                                        return Err(PyError::runtime_error(
                                            "expected BuiltinMethod for send",
                                        ))
                                    }
                                };
                                self.call_function(bound, vec![val], vec![])
                            }
                            Err(_) => Err(PyError::attribute_error("object has no send method")),
                        }
                    } else {
                        // Handle Instance objects and other types with a send method
                        match iter_val.borrow().get_attribute("send") {
                            Ok(send_method) => {
                                let bound = match &*send_method.borrow() {
                                    PyObject::BuiltinMethod { func, .. } => {
                                        PyObjectRef::imm(PyObject::BuiltinMethod {
                                            name: "send".to_string(),
                                            func: *func,
                                            self_obj: iter_val.clone(),
                                        })
                                    }
                                    _ => {
                                        return Err(PyError::runtime_error(
                                            "expected BuiltinMethod for send",
                                        ))
                                    }
                                };
                                self.call_function(bound, vec![val], vec![])
                            }
                            Err(_) => {
                                // No send method — try __next__ (for simple iterators used with await)
                                Err(PyError::type_error(
                                    "SEND on non-generator/coroutine/instance",
                                ))
                            }
                        }
                    }
                };
                match result {
                    Ok(val) => {
                        self.frames[fi].push(val);
                    }
                    Err(e) => {
                        match e {
                            PyError::StopIteration => {
                                // StopIteration with no value — push None as return value
                                self.frames[fi].push(py_none());
                                // Jump to cleanup target (absolute jump, like FOR_ITER)
                                self.frames[fi].ip = arg as usize;
                            }
                            PyError::Exception(ref typ, ref _exc_val) if typ == "StopIteration" => {
                                // Extract the return value from the PyError::Exception
                                let return_val = _exc_val.clone();
                                self.frames[fi].push(return_val);
                                // Jump to cleanup target (absolute jump, like FOR_ITER)
                                self.frames[fi].ip = arg as usize;
                            }
                            other => return Err(other),
                        }
                    }
                }
            }

            Opcode::END_SEND => {
                // Pop result and iterator, push result (validates proper stack state)
                let result = self.frames[fi].pop()?;
                let _iter = self.frames[fi].pop()?; // iterator, discarded
                self.frames[fi].push(result);
            }

            Opcode::CLEANUP_THROW => {
                // Cleanup after a throw into a generator
                // For now, just a no-op that handles cleanup
                self.frames[fi].pop()?;
            }

            Opcode::ELSE => {
                // No-op marker: separates except handlers from else block.
                // The compiler emits this so the exception table knows where
                // the else block starts.
            }

            Opcode::END_FINALLY => {
                // End of finally block. The stack has either:
                //   [..., value]  — normal execution (no exception)
                //   [..., exc]    — exception was handled, just re-raise
                //   [..., None]   — exception was suppressed/returned
                // We pop the top value. If it's an exception object, re-raise.
                match self.frames[fi].pop() {
                    Ok(val) => {
                        let is_exception = matches!(&*val.borrow(), PyObject::Exception { .. });
                        if is_exception {
                            return Err(PyError::Exception("".to_string(), val));
                        }
                        // Otherwise it was a normal value (or None) — continue
                    }
                    Err(e) => return Err(e),
                }
            }

            Opcode::POP_EXCEPT_AND_EXECUTE_FINALLY => {
                // Popped from POP_EXCEPT: the exception info was already popped.
                // Jump to the finally block address (stored in arg).
                // The finally block address is stored in the `arg` field.
                self.frames[fi].ip = arg as usize;
            }

            Opcode::CALL_FUNCTION_EX => {
                // f(*args, **kwargs) — the compiler already built a real
                // tuple (unpacking any starred arguments via LIST_EXTEND)
                // and a real dict (merging any bare **expr via DICT_MERGE),
                // so this just needs to unpack those into a normal call.
                let kwargs_dict = self.frames[fi].pop()?;
                let args_tuple = self.frames[fi].pop()?;
                let callable = self.frames[fi].pop()?;
                let args_vec = match &*args_tuple.borrow() {
                    PyObject::Tuple(v) | PyObject::List(v) => v.clone(),
                    _ => return Err(PyError::type_error("argument after * must be an iterable")),
                };
                let keywords_vec: Vec<(String, PyObjectRef)> = match &*kwargs_dict.borrow() {
                    PyObject::Dict(d) => d.items().into_iter().map(|(k, v)| (k.str(), v)).collect(),
                    _ => Vec::new(),
                };
                let result = self.call_function(callable, args_vec, keywords_vec)?;
                self.frames[fi].push(result);
            }

            _ => {
                return Err(PyError::runtime_error(format!(
                    "unimplemented opcode: {:?}",
                    op
                )))
            }
        }
        Ok(None)
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
                Some(rc) => PyObjectRef::Imm(rc),
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
                let target_ref = PyObjectRef::Imm(rc);
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

        #[cfg_attr(not(feature = "jit"), allow(unused_variables))]
        if let PyObject::Function(ref inner_f) = &*callable.borrow() {
            let code = &inner_f.code;
            let func_globals = &inner_f.globals;
            let defaults = &inner_f.defaults;
            let closure = &inner_f.closure;
            let jit_ptr = &inner_f.jit_ptr;
            let jit_consts = &inner_f.jit_consts;
            // Try JIT compiled execution (fast path for hot functions)
            #[cfg(feature = "jit")]
            if defaults.is_empty()
                && keywords.is_empty()
                && !crate::cycle_gc::IN_FINALIZER.with(std::cell::Cell::get)
            {
                const SENTINEL_FAILED: usize = 1;
                let jp = jit_ptr.get();
                if jp == SENTINEL_FAILED {
                    // A previous compile attempt failed — stick with the
                    // interpreter (don't retry on every call).
                } else {
                    if jp == 0 {
                        // First call: compile now and run the result
                        // immediately (this VM's tests call most functions
                        // once, so deferring to the second call would leave
                        // them interpreted forever).
                        let compiled_fn = self.jit.borrow_mut().compile(code);
                        match compiled_fn {
                            Some(compiled_fn) => {
                                let precomputed = crate::jit::JitCompiler::precompute_for_jit(
                                    code,
                                    func_globals,
                                    &self.builtins,
                                );
                                jit_ptr.set(compiled_fn as usize);
                                *jit_consts.borrow_mut() = precomputed;
                            }
                            None => {
                                jit_ptr.set(SENTINEL_FAILED);
                            }
                        }
                    }
                    let jp = jit_ptr.get();
                    if jp != 0 && jp != SENTINEL_FAILED {
                        // SAFETY: `jp` was just produced by
                        // `self.jit.borrow_mut().compile(code)` above (or on
                        // a prior call for the same `code`), which only ever
                        // emits machine code matching this exact
                        // `extern "C"` signature — the JIT codegen in
                        // jit.rs is the sole producer of values stored in
                        // `jit_ptr`.
                        let func_ptr: extern "C" fn(
                            *const PyObjectRef,
                            usize,
                            *const PyObjectRef,
                            *mut PyObjectRef,
                        ) = unsafe { std::mem::transmute(jp) };
                        let n = args.len().min(code.arg_count as usize);
                        let mut fast_locals: Vec<PyObjectRef> = Vec::with_capacity(n);
                        for i in 0..n {
                            fast_locals.push(args[i].clone());
                        }
                        let consts = jit_consts.borrow();
                        let mut result = PyObjectRef::None;
                        let _guard = crate::jit::set_jit_globals(func_globals.clone());
                        func_ptr(
                            fast_locals.as_ptr(),
                            fast_locals.len(),
                            consts.as_ptr(),
                            &mut result,
                        );
                        return Ok(result);
                    }
                }
            }

            // Try simple execution without Frame creation
            if defaults.is_empty() && keywords.is_empty() {
                if let Some(result) = Self::try_exec_simple(code, &args) {
                    return result;
                }
            }
            // A Python-level function call here recurses through actual
            // Rust call frames (`call_function` -> `execute()` ->
            // `execute_inner` -> `execute_instruction`'s `CALL` handling ->
            // `call_function` -> ...), with no equivalent of CPython's own
            // `sys.getrecursionlimit()` check anywhere — so unbounded
            // Python recursion (a plain accidental bug in user code, not
            // some contrived edge case) previously overflowed the REAL
            // native thread stack and hard-aborted the whole process
            // (`fatal runtime error: stack overflow`) instead of raising a
            // catchable `RecursionError`, exactly like real CPython does.
            // Confirmed general via the simplest possible repro (`def
            // f(n): return f(n+1)` called once) and via CPython's own
            // `test_isinstance.py`'s deliberate recursion-limit tests.
            // Reads `self.recursion_limit` (default matches real CPython's
            // `sys.getrecursionlimit()`, 1000 — see its own doc comment).
            // Made safe by `main.rs` running everything on a dedicated,
            // much larger-than-default stack sized with headroom to spare
            // even at the default limit.
            if self.frames.len() >= self.recursion_limit {
                return Err(PyError::recursion_error("maximum recursion depth exceeded"));
            }
            let func_globals = func_globals.clone();
            let defaults = defaults.clone();
            let code_rc = Rc::new(code.clone());
            let mut new_frame = self.acquire_frame(
                Rc::clone(&code_rc),
                func_globals,
                Rc::clone(&self.builtins),
                None,
            );
            new_frame.closure = Box::new(closure.clone());
            let code = code;

            let npos = args.len();
            let named_params = code.arg_count;
            let fname = interner::lookup_str(code.name).to_string();

            fn format_missing_names(names: &[String]) -> String {
                match names.len() {
                    0 => String::new(),
                    1 => format!("'{}'", names[0]),
                    2 => format!("'{}' and '{}'", names[0], names[1]),
                    _ => {
                        let (last, rest) = names.split_last().unwrap();
                        let joined = rest
                            .iter()
                            .map(|n| format!("'{}'", n))
                            .collect::<Vec<_>>()
                            .join(", ");
                        format!("{}, and '{}'", joined, last)
                    }
                }
            }

            // Real Python raises `TypeError` immediately when more positional
            // arguments are given than the function accepts (and it has no
            // `*args` to absorb the excess) — this whole argument-binding
            // block had NO validation of any kind before this fix: too many
            // positional args were silently dropped, missing required args
            // were never detected (the function body would just hit
            // `LOAD_FAST unbound` chaos or read `None`), unexpected keyword
            // arguments were silently inserted as a throwaway local name,
            // and a keyword colliding with an already-positionally-filled
            // parameter silently overwrote it instead of raising. Found via
            // CPython's own `test_call.py`
            // (`TestErrorMessagesUseQualifiedName`/`CFunctionCallsErrorMessages`),
            // whose whole point is exercising exactly these error paths —
            // every single one of them was a real, silent correctness bug
            // affecting EVERY user-defined function call in the interpreter.
            if npos > named_params && code.vararg_name.is_none() {
                self.release_frame(new_frame);
                let num_defaults = code.num_defaults;
                let min_required = named_params.saturating_sub(num_defaults);
                // CPython's arg-count TypeError grammar: the noun agrees
                // with the count, the verb agrees with the TOTAL (npos +
                // keyword-only) — e.g. "but 1 positional argument (and 1
                // keyword-only argument) were given". Matches the doctest in
                // test_extcall.py exactly.
                let noun = |n: usize| if n == 1 { "argument" } else { "arguments" };
                // count how many passed keywords target kwonly params, for the
                // extended error message "and N keyword-only arguments"
                let kwonly_given = if code.kwonlyarg_count > 0 && !keywords.is_empty() {
                    let kwonly_start_tmp =
                        code.arg_count + if code.vararg_name.is_some() { 1 } else { 0 };
                    let kwonly_names = &code.varnames
                        [kwonly_start_tmp..kwonly_start_tmp + code.kwonlyarg_count];
                    keywords
                        .iter()
                        .filter(|(k, _)| {
                            kwonly_names.iter().any(|&n| crate::interner::intern_eq(n, k))
                        })
                        .count()
                } else {
                    0
                };
                let msg = if kwonly_given > 0 {
                    // CPython 3.14: "takes X positional arguments but Y positional arguments (and Z keyword-only arguments) were given"
                    format!(
                        "{}() takes {} positional {} but {} positional {} (and {} keyword-only {}) were given",
                        fname,
                        named_params,
                        noun(named_params),
                        npos,
                        noun(npos),
                        kwonly_given,
                        noun(kwonly_given),
                    )
                } else if num_defaults == 0 {
                    format!(
                        "{}() takes {} positional {} but {} {} given",
                        fname,
                        named_params,
                        noun(named_params),
                        npos,
                        if npos == 1 { "was" } else { "were" }
                    )
                } else {
                    let verb = if npos > 1 { "were" } else { "was" };
                    format!(
                        "{}() takes from {} to {} positional arguments but {} {} given",
                        fname,
                        min_required,
                        named_params,
                        npos,
                        verb
                    )
                };
                return Err(PyError::type_error(msg));
            }

            // Assign positional args to named parameters
            for i in 0..npos.min(named_params) {
                let name_clone = new_frame.code.varnames[i].to_string();
                new_frame.insert_local(&name_clone, args[i].clone());
                if i < new_frame.fast_locals.len() {
                    new_frame.fast_locals[i] = Some(args[i].clone());
                }
            }

            // Pack excess positional args into *args
            if let Some(vararg_name) = &code.vararg_name {
                let mut extra = Vec::new();
                for i in named_params..npos {
                    extra.push(args[i].clone());
                }
                let vararg_val = py_tuple(extra);
                if let Some(idx) = new_frame
                    .code
                    .varnames
                    .iter()
                    .position(|&n| crate::interner::intern_eq(n, vararg_name))
                {
                    if idx < new_frame.fast_locals.len() {
                        new_frame.fast_locals[idx] = Some(vararg_val.clone());
                    }
                }
                new_frame.insert_local(&vararg_name, vararg_val);
            }

            // Apply defaults for missing positional params
            if npos < named_params {
                let num_defaults = code.num_defaults;
                // Parameters are split into two groups: those WITHOUT defaults (non-defaulted),
                // and those WITH defaults (defaulted). self (index 0) is never defaulted.
                // defaulted params start at index (named_params - num_defaults)
                let first_default = named_params - num_defaults;
                for i in npos..named_params {
                    if i >= first_default {
                        let default_idx = i - first_default;
                        let name_clone = new_frame.code.varnames[i].to_string();
                        let val = if default_idx < defaults.len() {
                            defaults[default_idx].clone()
                        } else {
                            py_none()
                        };
                        new_frame.insert_local(&name_clone, val.clone());
                        if i < new_frame.fast_locals.len() {
                            new_frame.fast_locals[i] = Some(val);
                        }
                    }
                }
            }

            // Handle **kwargs
            let kwonly_start = code.arg_count + if code.vararg_name.is_some() { 1 } else { 0 };
            let positional_filled = npos.min(named_params);
            if let Some(kwarg_name) = &code.kwarg_name {
                let kw_dict = py_dict();
                for (key, value) in &keywords {
                    if let Some(idx) = formal_param_index(
                        &new_frame.code.varnames,
                        code.arg_count,
                        code.posonlyarg_count,
                        code.kwonlyarg_count,
                        kwonly_start,
                        key,
                    ) {
                        // A keyword targeting a positional-only param goes
                        // into **kwargs (real Python: `f(42, a=1)` with `a`
                        // posonly lands in kwargs, never on the param).
                        if idx < code.posonlyarg_count {
                            if let PyObject::Dict(ref mut dict) = &mut *kw_dict.borrow_mut() {
                                dict.set(py_str(key), value.clone())?;
                            }
                            continue;
                        }
                        // A keyword targeting a formal parameter that ALREADY
                        // received a positional value — real Python's
                        // `TypeError: ...() got multiple values for argument
                        // '...'`, previously silently overwritten.
                        if idx < positional_filled {
                            self.release_frame(new_frame);
                            return Err(PyError::type_error(format!(
                                "{}() got multiple values for argument '{}'",
                                fname, key
                            )));
                        }
                        new_frame.insert_local(&key, value.clone());
                        if idx < new_frame.fast_locals.len() {
                            new_frame.fast_locals[idx] = Some(value.clone());
                        }
                    } else {
                        if let PyObject::Dict(ref mut dict) = &mut *kw_dict.borrow_mut() {
                            // A key supplied more than once — via `**{k: v}`
                            // expansion AND an explicit keyword (or twice via
                            // **) — is `TypeError: ...() got multiple values
                            // for keyword argument 'k'` (test_extcall's
                            // doctest: `f(1, 2, **{'a': -1}, a=4, c=6)`).
                            if dict.get(&py_str(key)).ok().flatten().is_some() {
                                self.release_frame(new_frame);
                                return Err(PyError::type_error(format!(
                                    "{}() got multiple values for keyword argument '{}'",
                                    fname, key
                                )));
                            }
                            dict.set(py_str(key), value.clone())?;
                        }
                    }
                }
                if let Some(idx) = new_frame
                    .code
                    .varnames
                    .iter()
                    .position(|n| crate::interner::lookup_str(*n) == kwarg_name.as_str())
                {
                    if idx < new_frame.fast_locals.len() {
                        new_frame.fast_locals[idx] = Some(kw_dict.clone());
                    }
                }
                new_frame.insert_local(kwarg_name.as_str(), kw_dict);
            } else {
                // No **kwargs: keyword args must still bind to the matching
                // named parameter's FAST local slot (LOAD_FAST reads
                // fast_locals, not the insert_local name dict — missing this
                // meant `f(1, somekw=True)` left `somekw` as None in
                // fast_locals, raising "referenced before assignment" the
                // moment the function body read it), matching the
                // **kwargs branch above. A keyword matching no formal
                // parameter, or one that already got a positional value,
                // must raise `TypeError` — previously silently accepted as
                // either a no-op or a throwaway local-name insertion the
                // function body never referenced.
                // With no **kwargs, ALL keywords targeting positional-only
                // params are reported together (real Python's
                // "got some positional-only arguments passed as keyword
                // arguments: 'a, b'").
                let posonly_keywords: Vec<&String> = keywords
                    .iter()
                    .filter_map(|(k, _)| {
                        formal_param_index(
                            &new_frame.code.varnames,
                            code.arg_count,
                            code.posonlyarg_count,
                            code.kwonlyarg_count,
                            kwonly_start,
                            k,
                        )
                        .filter(|idx| *idx < code.posonlyarg_count)
                        .map(|_| k)
                    })
                    .collect();
                if !posonly_keywords.is_empty() {
                    self.release_frame(new_frame);
                    let names = posonly_keywords
                        .iter()
                        .map(|k| k.as_str())
                        .collect::<Vec<_>>()
                        .join(", ");
                    return Err(PyError::type_error(format!(
                        "{}() got some positional-only arguments passed as keyword arguments: '{}'",
                        fname, names
                    )));
                }
                for (key, value) in &keywords {
                    match formal_param_index(
                        &new_frame.code.varnames,
                        code.arg_count,
                        code.posonlyarg_count,
                        code.kwonlyarg_count,
                        kwonly_start,
                        key,
                    ) {
                        Some(idx) if idx < code.posonlyarg_count => {
                            // Unreachable (pre-scanned above) — keep for safety.
                            self.release_frame(new_frame);
                            return Err(PyError::type_error(format!("{}() got some positional-only arguments passed as keyword arguments: '{}'", fname, key)));
                        }
                        Some(idx) if idx < positional_filled => {
                            self.release_frame(new_frame);
                            return Err(PyError::type_error(format!(
                                "{}() got multiple values for argument '{}'",
                                fname, key
                            )));
                        }
                        Some(idx) => {
                            if idx < new_frame.fast_locals.len() {
                                new_frame.fast_locals[idx] = Some(value.clone());
                            }
                            new_frame.insert_local(&key, value.clone());
                        }
                        None => {
                            self.release_frame(new_frame);
                            return Err(PyError::type_error(format!(
                                "{}() got an unexpected keyword argument '{}'",
                                fname, key
                            )));
                        }
                    }
                }
            }

            // Apply defaults for still-unbound keyword-only params (CPython's
            // __kwdefaults__ equivalent) — must run after explicit keyword
            // binding above, since only truly-unbound kwonly slots should
            // get their default. Defaults for kwonly params live in
            // `defaults` right after the positional ones (see
            // CodeObject::kwonly_defaults_mask / MAKE_FUNCTION).
            if code.kwonlyarg_count > 0 {
                // A live `__kwdefaults__` dict set on the function (either the
                // default one or a REPLACEMENT — `f.__kwdefaults__ = {...}`
                // must affect subsequent calls, test_keywordonlyarg's
                // testKwDefaults) is the source of truth for kwonly defaults,
                // overriding the compiled-in ones.
                let live_kwdefaults: Option<Box<crate::object::PyDict>> =
                    inner_f.dict.get("__kwdefaults__").and_then(|v| {
                        if let PyObject::Dict(d) = &*v.borrow() {
                            Some(d.clone())
                        } else {
                            None
                        }
                    });
                let kwonly_start = code.arg_count + if code.vararg_name.is_some() { 1 } else { 0 };
                // Build the name -> default map FIRST by consuming the
                // compiled-in defaults list sequentially over the FULL
                // kwonly parameter list. Applying per-slot while iterating
                // skipped explicitly-bound params WITHOUT consuming their
                // default, shifting every later default onto the wrong
                // parameter (observed: ConfigParser(interpolation=<bool>)).
                let name_to_default: std::collections::HashMap<String, PyObjectRef> =
                    match &live_kwdefaults {
                        Some(d) => d
                            .items()
                            .into_iter()
                            .filter_map(|(k, v)| Some((k.str(), v)))
                            .collect(),
                        None => {
                            let mut m = std::collections::HashMap::new();
                            let mut idx = code.num_defaults;
                            for (k, has_default) in code.kwonly_defaults_mask.iter().enumerate() {
                                let _ = k;
                                if *has_default {
                                    if let Some(v) = defaults.get(idx).cloned() {
                                        let name_str = interner::lookup_str(
                                            new_frame.code.varnames[kwonly_start + k],
                                        )
                                        .to_string();
                                        m.insert(name_str, v);
                                    }
                                    idx += 1;
                                }
                            }
                            m
                        }
                    };
                for k in 0..code.kwonly_defaults_mask.len() {
                    let idx = kwonly_start + k;
                    if idx >= new_frame.fast_locals.len()
                        || new_frame.fast_locals[idx].is_some()
                    {
                        continue;
                    }
                    let name_str =
                        interner::lookup_str(new_frame.code.varnames[idx]).to_string();
                    if let Some(val) = name_to_default.get(&name_str) {
                        new_frame.insert_local(&name_str, val.clone());
                        new_frame.fast_locals[idx] = Some(val.clone());
                    }
                }
            }

            // Any formal positional/keyword-only parameter still unbound at
            // this point has no value at all — real Python's `TypeError:
            // ...() missing N required positional/keyword-only argument(s):
            // '...'`, previously never checked.
            let missing_positional: Vec<String> = (0..named_params)
                .filter(|&i| i >= new_frame.fast_locals.len() || new_frame.fast_locals[i].is_none())
                .map(|i| interner::lookup_str(new_frame.code.varnames[i]).to_string())
                .collect();
            if !missing_positional.is_empty() {
                self.release_frame(new_frame);
                let n = missing_positional.len();
                return Err(PyError::type_error(format!(
                    "{}() missing {} required positional argument{}: {}",
                    fname,
                    n,
                    if n == 1 { "" } else { "s" },
                    format_missing_names(&missing_positional)
                )));
            }
            let missing_kwonly: Vec<String> = (kwonly_start..kwonly_start + code.kwonlyarg_count)
                .filter(|&i| i >= new_frame.fast_locals.len() || new_frame.fast_locals[i].is_none())
                .map(|i| interner::lookup_str(new_frame.code.varnames[i]).to_string())
                .collect();
            if !missing_kwonly.is_empty() {
                self.release_frame(new_frame);
                let n = missing_kwonly.len();
                return Err(PyError::type_error(format!(
                    "{}() missing {} required keyword-only argument{}: {}",
                    fname,
                    n,
                    if n == 1 { "" } else { "s" },
                    format_missing_names(&missing_kwonly)
                )));
            }

            self.push_frame(new_frame);
            let result = self.execute();
            if let Some(frame) = self.frames.pop() {
                self.release_frame(frame);
            }
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

        // Calling a METACLASS itself with `(name, bases, namespace)` — e.g.
        // `Meta('D', (object,), {})`, the dynamic equivalent of
        // `class D(metaclass=Meta): ...` — must build a CLASS whose
        // metaclass is `Meta`. Real Python routes this through
        // `type.__call__` → `type.__new__`, which recognizes that the
        // callable is not bare `type` and performs metaclass-tagged class
        // construction. Without this, every dynamically created custom-
        // metaclass class silently lost its metaclass (its `__dict__` even
        // came up empty — real casualties: `typing._ProtocolMeta` protocol
        // classes, `abc.ABCMeta` registries, enum's `EnumType` when built
        // dynamically). Only the exact 3-arg shape is intercepted so
        // ordinary instantiation of such classes is untouched.
        {
            let looks_like_class_call = args.len() == 3
                && matches!(&*args[0].borrow(), PyObject::Str(_))
                && matches!(&*args[2].borrow(), PyObject::Dict(_));
            if std::env::var("RPY_TRACE_MC").is_ok() {
                let tn = callable.borrow().type_name();
                let mt = crate::object::metatype_of(&callable).map(|m| m.borrow().type_name());
                eprintln!("MC-TRACE type-call: callable={} args={} looks3={} mt={:?}",
                          tn, args.len(), looks_like_class_call, mt);
            }
            if looks_like_class_call {
                let plain_type = self
                    .builtins
                    .get(&interner::intern("type"))
                    .cloned();
                let callable_is_bare_type =
                    plain_type.as_ref().map(|t| t.is(&callable)).unwrap_or(false);
                // The callable must itself be a SUBCLASS of `type` (its MRO
                // contains bare `type`) — i.e. it's a metaclass. NOTE: do
                // NOT gate this on `metatype_of(callable)` being custom: a
                // metaclass declared via `class M(type): ...` has plain
                // `type` as ITS OWN metaclass, so that lookup is None even
                // though calling M must build a class tagged with M.
                let callable_is_metaclass = matches!(
                    &*callable.borrow(),
                    PyObject::Type { mro, .. } if plain_type
                        .as_ref()
                        .map(|t| mro.iter().any(|b| t.is(b)))
                        .unwrap_or(false)
                );
                if !callable_is_bare_type && callable_is_metaclass {
                    let mut new_args = vec![callable.clone()];
                    new_args.extend(args.iter().cloned());
                    if !keywords.is_empty() {
                        let mut d = crate::object::PyDict::new();
                        for (k, v) in &keywords {
                            d.set(crate::object::py_str(k), v.clone())?;
                        }
                        new_args.push(crate::object::PyObjectRef::new(crate::object::PyObject::Dict(Box::new(d))));
                    }
                    let built = self.type_new_impl(&new_args)?;
                    if std::env::var("RPY_TRACE_MC").is_ok() {
                        let tn = built.borrow().type_name();
                        let dn = if let PyObject::Type { dict, .. } = &*built.borrow() {
                            format!("{:?}", dict.get_str("marker").is_some())
                        } else { "?".into() };
                        eprintln!("MC-TRACE built via type_new_impl: type={} has_marker={}", tn, dn);
                    }
                    return Ok(built);
                }
            }
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

        let type_construct_info = if let PyObject::Type { dict, mro, .. } = &*callable.borrow() {
            let native_kind = dict
                .get_str(crate::object::NATIVE_BASE_MARKER)
                .map(|v| v.str());
            let init_func = dict.get_str("__init__").cloned().or_else(|| {
                for base in mro.iter().skip(1) {
                    if let PyObject::Type {
                        name: base_name,
                        dict: base_dict,
                        ..
                    } = &*base.borrow()
                    {
                        // Every class implicitly inherits from `object`,
                        // whose own __init__ is a universal no-op. For a
                        // class that also has a native base (e.g.
                        // `class SafeString(str, SafeData): ...`), that
                        // no-op would otherwise always be found first and
                        // preempt real native construction — skip it here
                        // so synthesize_native_init below gets a chance
                        // unless something more specific actually overrides
                        // __init__.
                        if native_kind.is_some() && base_name == "object" {
                            continue;
                        }
                        if let Some(val) = base_dict.get_str("__init__") {
                            return Some(val.clone());
                        }
                    }
                }
                None
            });
            Some((native_kind, init_func))
        } else {
            None
        };
        // The `callable.borrow()` above must be dropped (it already is, by
        // this point — the `if let` scrutinee's temporary ends with the
        // `if let` expression) before calling `__init__` below: `__init__`'s
        // body commonly references its own class by name (e.g. a
        // class-level counter like `Field.creation_counter += 1`, a
        // widespread real-world pattern, not specific to any one
        // library) — a STORE_ATTR on `callable` while this function still
        // held it borrowed here was a genuine double-borrow panic.
        if let Some((native_kind, init_func)) = type_construct_info {
            // A user-defined `__new__` (a Python Function, not the native
            // float/int/... `__new__`) must be called and its result
            // returned (class Foo3(float): def __new__(...): return
            // float.__new__(cls, 2*value) — Foo3(21) == 42). The native
            // __new__ on the base type builds the default instance.
            let custom_new = crate::object::lookup_dunder_via_mro(&callable, "__new__")
                .filter(|f| matches!(&*f.borrow(), PyObject::Function(_)));
            if let Some(new_fn) = custom_new {
                let mut new_args = args.clone();
                new_args.insert(0, callable.clone());
                let kw_clone = keywords.clone();
                let result = self.call_function(new_fn, new_args, kw_clone)?;
                // A user exception class whose `__new__` returns a
                // non-BaseException must raise TypeError (CPython: "calling
                // <class '...'> should have returned an instance of
                // BaseException, not <class 'list'>"). Without the check the
                // raw non-exception value would be raised/propagated and
                // escape every `except BaseException`.
                if crate::object::find_exception_base_name(&callable).is_some() {
                    let is_exc = match &*result.borrow() {
                        PyObject::Exception { .. } | PyObject::ExceptionGroup { .. } => true,
                        PyObject::Instance { typ, .. } => {
                            crate::object::find_exception_base_name(typ).is_some()
                        }
                        _ => false,
                    };
                    if !is_exc {
                        let result_typ = result.borrow().type_name();
                        return Err(PyError::type_error(format!(
                            "calling {} should have returned an instance of BaseException, not <class '{}'>",
                            callable.repr(),
                            result_typ
                        )));
                    }
                }
                // CPython: if __new__ returned an instance of this class,
                // AND __init__ is defined (and different from the base),
                // call __init__ before returning.
                // If __new__ returned an instance of this class AND __init__
                // is defined (and not the base object.__init__ no-op), call
                // __init__ — CPython's type_call always does this when
                // isinstance(result, cls) is true.
                let r = result.borrow();
                let is_instance_of_class = match &*r {
                    PyObject::Instance { typ, .. } => {
                        if typ.is(&callable) {
                            true
                        } else if let PyObject::Type { mro, .. } = &*typ.borrow() {
                            mro.iter().any(|b| b.is(&callable))
                        } else {
                            false
                        }
                    }
                    _ => false,
                };
                drop(r);
                if is_instance_of_class && init_func.is_some() {
                    let init_fn = init_func.clone().unwrap();
                    // Skip object.__init__ (universal no-op for native types)
                    let skip = matches!(&*init_fn.borrow(), PyObject::BuiltinFunction { name, .. } if name == "__init__");
                    if !skip {
                        let mut init_args = args.clone();
                        init_args.insert(0, result.clone());
                        self.call_function(init_fn, init_args, keywords.clone())?;
                    }
                }
                return Ok(result);
            }
            // ABC enforcement: if the class has __abstractmethods__ that is
            // non-empty, instantiation must raise TypeError (CPython:
            // "Can't instantiate abstract class ... with abstract methods").
            let abstracts_opt: Option<PyObjectRef> = (|| {
                match callable.borrow().get_attribute("__abstractmethods__") {
                    Ok(v) => Some(v),
                    Err(_) => None,
                }
            })();
            if let Some(abstracts) = abstracts_opt {
                let n = match &*abstracts.borrow() {
                    PyObject::FrozenSet(s) => s.len(),
                    PyObject::Set(s) => s.len(),
                    _ => 0,
                };
                if n > 0 {
                    // Collect the abstract method names for the error message.
                    let names: Vec<String> = match &*abstracts.borrow() {
                        PyObject::FrozenSet(s) => s.iter().map(|v| v.str()).collect(),
                        PyObject::Set(s) => s.iter().map(|v| v.str()).collect(),
                        _ => vec![],
                    };
                    let mut sorted = names;
                    sorted.sort();
                    return Err(PyError::type_error(format!(
                        "Can't instantiate abstract class {} with abstract method{} {}",
                        callable.borrow().type_name(),
                        if sorted.len() == 1 { "" } else { "s" },
                        sorted.join(", ")
                    )));
                }
            }

            let mut instance_dict = AttrMap::new();
            if let Some(kind) = &native_kind {
                instance_dict.insert(
                    crate::object::NATIVE_BACKING_KEY.to_string(),
                    crate::object::make_native_backing(kind),
                );
            }
            let instance = PyObjectRef::new(PyObject::Instance {
                typ: callable.clone(),
                dict: instance_dict,
            });
            // The native VALUE comes from `__new__(cls, *args)` — CPython
            // builds it BEFORE `__init__` runs, so even a custom `__init__`
            // (which overrides the native float/int/... init) must NOT leave
            // the backing at its default (`class Foo(float): def __init__
            // (self, x, ...): ...; Foo(2.5)` is still 2.5 — test_float's
            // test_keywords_in_subclass). Synthesize from the constructor
            // args unconditionally when there's a native base.
            if let Some(kind) = &native_kind {
                // A CONTAINER subclass (`class Counter(dict)`, `class
                // MyList(list)`) with a custom Python `__init__` is
                // different: `dict.__new__`/`list.__new__` ignore the
                // constructor args (the backing starts EMPTY) and the
                // custom `__init__` is what populates it (e.g.
                // `Counter('aabbc')` counts via its own `update`). Building
                // the backing from the args first (`builtin_dict('aabbc')`)
                // raises "cannot convert dictionary update sequence
                // element to a sequence" before `__init__` ever runs.
                let custom_py_init =
                    matches!(&init_func, Some(f) if matches!(&*f.borrow(), PyObject::Function(_)));
                let is_mutable_container = matches!(
                    kind.as_str(),
                    "dict" | "list" | "set" | "deque" | "bytearray"
                );
                let native = if custom_py_init && is_mutable_container {
                    crate::object::make_native_backing(kind)
                } else if custom_py_init
                    && matches!(
                        kind.as_str(),
                        "tuple" | "frozenset" | "bytes" | "str" | "int" | "float" | "complex"
                    )
                {
                    // Immutable base: its value is created by __new__, not
                    // __init__. When a subclass overrides __init__ with extra
                    // args (e.g. `class S(tuple): def __init__(self, arg,
                    // newarg=None)` → `S([1,2], newarg=3)`), those extra
                    // args belong to __init__, not to tuple.__new__. CPython's
                    // type_call slices them: __new__ receives only the
                    // iterable, __init__ receives the full args. Without this,
                    // passing extra kwargs to synthesize would either raise
                    // "tuple() takes no keyword arguments" incorrectly or,
                    // if skipped entirely, leave the backing empty (the
                    // observed [] vs [1,2] failure in
                    // test_keywords_in_subclass).
                    let truncated_args: &[PyObjectRef] =
                        if args.is_empty() { &[] } else { &args[0..1] };
                    crate::object::synthesize_native_init(kind, truncated_args, &[])?
                } else {
                    crate::object::synthesize_native_init(kind, &args, &keywords)?
                };
                if let PyObject::Instance { dict, .. } = &mut *instance.borrow_mut() {
                    dict.insert(crate::object::NATIVE_BACKING_KEY.to_string(), native);
                }
            } else if init_func.is_none()
                && crate::object::find_exception_base_name(&callable).is_some()
            {
                // `class MyError(Exception): pass` (no explicit __init__) —
                // real Python's `BaseException.__init__` always stores
                // `self.args = args`, which is what `str(exc)`/`repr(exc)`
                // and every uncaught-exception traceback print. Exception
                // builtins (Exception, ValueError, ...) are
                // `BuiltinFunction`s, not `PyObject::Type`s, so they never
                // appear in `mro` and were completely invisible to this
                // constructor logic — ANY user-defined exception subclass
                // (an extremely common, foundational pattern) silently got
                // no `args` at all, surfacing as "MyError: " (empty message)
                // or "Exception: re-raise" (the internal dispatch tag)
                // instead of the real message whenever it passed through a
                // `with`/`finally` or propagated uncaught.
                if let PyObject::Instance { dict, .. } = &mut *instance.borrow_mut() {
                    dict.insert_str("args", py_tuple(args.clone()));
                }
            }
            if let Some(init_func) = init_func {
                // Delegate to the real call_function instead of a hand-rolled
                // frame setup per callable kind — the latter (kept here for
                // a long time) never handled *args/**kwargs/default values at
                // all, silently binding missing parameters to None instead of
                // their real defaults and dropping every keyword argument
                // passed to the constructor. call_function already gets all
                // of that right for every callable variant (BuiltinFunction,
                // Function, Closure, ...).
                let mut init_args = vec![instance.clone()];
                init_args.extend(args);
                self.call_function(init_func, init_args, keywords)?;
            }
            return Ok(instance);
        }

        if let PyObject::BuildClass = &*callable.borrow() {
            if args.len() < 3 {
                return Err(PyError::type_error(
                    "__build_class__: need at least 3 arguments",
                ));
            }
            let func = args[0].clone();
            let name = args[1].clone();
            let bases = args[2].clone();

            let name_str = match &*name.borrow() {
                PyObject::Str(s) => s.to_string(),
                _ => return Err(PyError::type_error("class name must be a string")),
            };

            // Bases and any explicit `metaclass=` are already fully
            // evaluated by the time __build_class__ is called (the
            // compiler evaluates func/name/*bases/**kwds before emitting
            // the CALL — see Stmt::ClassDef's compilation) — so the
            // effective metaclass can and must be determined BEFORE the
            // class body executes, not after: a metaclass's `__prepare__`
            // (e.g. enum's `_EnumDict`-returning one, needed for Django's
            // `ChoicesType` to see a real `_member_names` list) has to
            // exist as the body's own namespace target from the very
            // start, not spliced in afterward.
            let explicit_metaclass = keywords
                .iter()
                .find(|(k, _)| k == "metaclass")
                .map(|(_, v)| v.clone());

            let bases_vec = if matches!(&*bases.borrow(), PyObject::None) {
                vec![]
            } else if let PyObject::Tuple(t) = &*bases.borrow() {
                t.clone()
            } else {
                vec![bases.clone()]
            };
            // Classes without explicit bases implicitly inherit from object
            let bases_vec = if bases_vec.is_empty() {
                // Look up 'object' type from builtins
                let object_type = self
                    .builtins
                    .get(&interner::intern("object"))
                    .cloned()
                    .unwrap_or_else(|| {
                        // Fallback: create a minimal object type
                        let mut obj_dict: TypeDict = Default::default();
                        obj_dict.insert_str(
                            "__setattr__",
                            PyObjectRef::new(PyObject::BuiltinFunction {
                                name: "__setattr__".to_string(),
                                func: |args| {
                                    if args.len() < 3 {
                                        return Err(PyError::type_error(
                                            "__setattr__ needs 3 args",
                                        ));
                                    }
                                    args[0]
                                        .borrow_mut()
                                        .set_attribute(&args[1].str(), args[2].clone())?;
                                    Ok(py_none())
                                },
                            }),
                        );
                        PyObjectRef::new(PyObject::Type {
                            name: "object".to_string(),
                            dict: Box::new(obj_dict),
                            bases: vec![],
                            mro: vec![],
                        })
                    });
                vec![object_type]
            } else {
                bases_vec
            };

            // __init_subclass__ (and any custom metaclass __new__/__init__)
            // only ever see the non-`metaclass` keywords.
            let init_subclass_kwargs: Vec<(String, PyObjectRef)> = keywords
                .iter()
                .filter(|(k, _)| k != "metaclass")
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();

            // Metaclass inheritance: without an explicit `metaclass=`, a
            // subclass of a class built by a custom metaclass must still be
            // built by that SAME metaclass (e.g. `class IntegerChoices(Choices,
            // IntEnum)` has no `metaclass=` of its own but must still use
            // `ChoicesType`, inherited from `Choices`) — matching CPython's
            // `_calculate_meta`. Real conflict resolution across multiple
            // unrelated custom metaclasses among the bases is out of scope
            // here (rare in practice); the first one found wins.
            let inherited_metaclass = bases_vec.iter().find_map(crate::object::metatype_of);
            let effective_metaclass = explicit_metaclass.or(inherited_metaclass);

            // If the effective metaclass defines `__prepare__`, call it now
            // (before the class body runs) to get the namespace object the
            // body's names should end up copied into. `__prepare__` is a
            // classmethod by convention — bind `mc` as its first arg
            // manually (mirroring StaticMethod/ClassMethod unwrapping used
            // elsewhere for `__new__`), since `lookup_dunder_via_mro`
            // itself does no descriptor binding.
            let prepared_namespace: Option<PyObjectRef> = if let Some(mc) = &effective_metaclass {
                crate::object::lookup_dunder_via_mro(mc, "__prepare__").and_then(|prep_fn| {
                    let unwrapped = match &*prep_fn.borrow() {
                        PyObject::ClassMethod { func } => func.clone(),
                        PyObject::StaticMethod { func } => func.clone(),
                        _ => prep_fn.clone(),
                    };
                    let is_classmethod = matches!(&*prep_fn.borrow(), PyObject::ClassMethod { .. });
                    let call_args = if is_classmethod {
                        vec![mc.clone(), name.clone(), bases.clone()]
                    } else {
                        vec![name.clone(), bases.clone()]
                    };
                    self.call_function(unwrapped, call_args, vec![]).ok()
                })
            } else {
                None
            };

            let namespace: Rc<RefCell<HashMap<StrId, PyObjectRef>>> =
                Rc::new(RefCell::new(HashMap::new()));
            let name_order = Rc::new(RefCell::new(Vec::new()));

            // Capture the calling frame's module_globals (or globals as fallback)
            // so that LOAD_NAME inside the class body can resolve module-level names.
            let caller_module_globals = if self.frames.len() >= 1 {
                let caller_frame = &self.frames[self.frames.len() - 1];
                caller_frame
                    .module_globals
                    .clone()
                    .or_else(|| Some(caller_frame.globals.clone()))
            } else {
                None
            };

            let mut class_cell: Option<PyObjectRef> = None;
            match &*func.borrow() {
                PyObject::Function(ref f) => {
                    let code = &f.code;
                    let closure = &f.closure;
                    let code = code.clone();
                    let closure = closure.clone();
                    let mut new_frame = self.acquire_frame(
                        code,
                        namespace.clone(),
                        Rc::clone(&self.builtins),
                        caller_module_globals,
                    );
                    new_frame.closure = Box::new(closure);
                    new_frame.name_order = Some(name_order.clone());
                    self.push_frame(new_frame);
                    // Must pop this frame unconditionally, including on
                    // error — `self.execute()?` used to return early on a
                    // class body raising mid-execution (e.g. a metaclass's
                    // `__init_subclass__`/descriptor processing failing),
                    // skipping the pop below and leaking the frame. Once
                    // leaked, `self.frames` never returns to the depth
                    // `handle_exception`'s `frame_floor` invariant assumes
                    // (exactly one frame per still-live `execute()` call),
                    // so a *later*, unrelated caught exception elsewhere
                    // finds the stack in a corrupted shape and blows up
                    // with "stack underflow (peek)" — confirmed via a
                    // frame_floor/frames.len() trace showing frames.len()
                    // staying flat across a frame_floor transition where it
                    // should have dropped by one.
                    let result = self.execute();
                    // PEP 3135: capture the class body's `__class__` cell so
                    // it can be populated with the finished class (methods
                    // created here close over it as a free var, letting bare
                    // super() resolve the class). The frame is released back
                    // to the pool right after, which clears fast_locals, so
                    // the cell must be grabbed now.
                    class_cell = {
                        let popped = self.frames.pop();
                        let cell = popped.as_ref().and_then(|fr| {
                            let idx = fr
                                .code
                                .varnames
                                .iter()
                                .position(|&n| crate::interner::lookup_str(n) == "__class__");
                            idx.and_then(|i| fr.fast_locals.get(i).and_then(|v| v.clone()))
                        });
                        if let Some(frame) = popped {
                            self.release_frame(frame);
                        }
                        cell
                    };
                    result?;
                }
                _ => return Err(PyError::type_error("class body must be a function")),
            }

            let namespace_dict: HashMap<String, PyObjectRef> = namespace
                .borrow()
                .iter()
                .map(|(k, v)| (interner::lookup_str(*k).to_string(), v.clone()))
                .collect();
            let order = name_order.borrow().clone();

            // If `__prepare__` produced a namespace object, replay the body's
            // assignments into it in definition order via real `[key] =
            // value` subscript-assignment — this is what actually invokes
            // e.g. `_EnumDict.__setitem__` for each name, letting it build
            // up its own tracking (like `_member_names`) exactly as if the
            // class body had written directly into it. Doing this as a
            // replay *after* the body runs (rather than making STORE_NAME
            // itself dict-object-aware) keeps class body execution
            // unchanged for the overwhelming common case (no `__prepare__`)
            // at the cost of not supporting code that reads its own
            // not-yet-assigned names back out mid-body through the custom
            // namespace's own __getitem__ — not needed here.
            //
            // Deliberately does NOT use the free `py_setitem` function here
            // (unlike STORE_SUBSCR) — that dispatches a found `__setitem__`
            // via `call_bound_method`, which (a separate, pre-existing,
            // documented limitation) spins up a brand-new disposable
            // `VirtualMachine::new()` for the call. Since this code runs
            // during `install_source_defined_stdlib`'s enum bootstrap
            // (i.e. *during* a VM's own construction), that disposable VM's
            // construction re-runs the exact same enum bootstrap, which
            // hits this exact same replay again — genuine infinite
            // recursion (confirmed via gdb backtrace showing repeated
            // VirtualMachine::new() frames), not just wasted work. `self`
            // is already the one real, live VM here, so call its own
            // `call_function` directly instead.
            if let Some(prepared) = &prepared_namespace {
                let setitem_fn = if let PyObject::Instance { typ, .. } = &*prepared.borrow() {
                    crate::object::lookup_dunder_via_mro(typ, "__setitem__")
                } else {
                    None
                };
                if std::env::var("RPY_DEBUG_METACLASS").is_ok() {
                    eprintln!(
                        "prepare-replay: name={} order={:?} has_setitem={}",
                        name_str,
                        order,
                        setitem_fn.is_some()
                    );
                }
                for k in &order {
                    if let Some(v) = namespace_dict.get(k) {
                        if std::env::var("RPY_DEBUG_METACLASS").is_ok() {
                            eprintln!("  replaying key={} value={}", k, v.repr());
                        }
                        if let Some(f) = &setitem_fn {
                            self.call_function(
                                f.clone(),
                                vec![prepared.clone(), py_str(k), v.clone()],
                                vec![],
                            )?;
                        } else if let Some(native) = crate::object::native_backing_of(prepared) {
                            if let PyObject::Dict(pd) = &mut *native.borrow_mut() {
                                pd.set(py_str(k), v.clone())?;
                            }
                        }
                    }
                }
                if std::env::var("RPY_DEBUG_METACLASS").is_ok() {
                    if let Some(native) = crate::object::native_backing_of(prepared) {
                        if let PyObject::Dict(pd) = &*native.borrow() {
                            eprintln!(
                                "  final native dict keys: {:?}",
                                pd.keys().iter().map(|k| k.str()).collect::<Vec<_>>()
                            );
                        }
                    }
                }
            }

            let class_result = if let Some(mc) = effective_metaclass {
                self.build_class_with_metaclass(
                    name_str,
                    name.clone(),
                    bases_vec,
                    namespace_dict,
                    order,
                    mc,
                    init_subclass_kwargs,
                    prepared_namespace,
                )
            } else {
                self.default_build_class(
                    name_str,
                    bases_vec,
                    namespace_dict,
                    init_subclass_kwargs,
                    None,
                )
            };
            let class_obj = class_result?;
            // PEP 3135: populate the class body's `__class__` cell with the
            // finished class, so bare `super()` in any method created in the
            // body resolves it (methods close over this cell as a free var).
            if let Some(cell) = class_cell {
                if let PyObject::Cell { value } = &mut *cell.borrow_mut() {
                    *value = Some(class_obj.clone());
                }
            }
            return Ok(class_obj);
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
