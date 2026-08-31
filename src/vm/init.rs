use crate::interner::{self, StrId};
use crate::modules::*;
use crate::object::*;
use crate::vm::helpers::find_lib_dir;
use std::collections::HashMap;

/// Populate the standard library modules into `modules`.
pub(crate) fn register_native_modules(
    modules: &mut HashMap<String, PyObjectRef>,
    builtins: &HashMap<StrId, PyObjectRef>,
) {
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

        let mut socket_dict = create_socket_dict();
        crate::modules::patch_socket_exception_aliases(&mut socket_dict, builtins);
        modules.insert_str("socket", create_module("socket", socket_dict.clone()));
        modules.insert_str("_socket", create_module("_socket", socket_dict));

        let mut select_dict = create_select_dict();
        // `select.error is OSError` — same fix as socket.error just above
        // (create_select_dict built its own synthetic "OSError"-named
        // Type instead of using the real one; `assertIs` needs genuine
        // object identity, not just a matching name).
        if let Some(real_oserror) = builtins.get(&crate::interner::intern("OSError")).cloned() {
            select_dict.insert("error".to_string(), real_oserror);
        }
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

        // `_collections_abc`/`collections.abc`: NOT pre-registered here.
        // These now resolve to the real, vendored `Lib/_collections_abc.py`
        // (built through real `abc.ABCMeta`) via normal file-based import,
        // wired up post-construction by
        // `VirtualMachine::install_collections_abc_alias` (see vm.rs) the
        // same way real CPython's `collections/__init__.py` aliases it.
        // Pre-registering a native dict here (as this used to do) would
        // shadow that real module and its ABCMeta-derived mixin methods.

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

        // `abc` and `_py_abc` are NOT registered as native modules — both now
        // load from `Lib/abc.py` and `Lib/_py_abc.py` (real CPython 3.14
        // sources, vendored verbatim) via the normal import machinery.
        // `Lib/abc.py` tries `from _abc import (...)` (the C accelerator)
        // first; since this codebase has no working native `_abc` module,
        // that import genuinely fails and it falls back to `from _py_abc
        // import ABCMeta, get_cache_token` — exactly real CPython's own
        // fallback path when built without the C extension. Previously both
        // names were aliased to the same broken native Rust stub
        // (`create_abc_dict`, since removed) whose `ABCMeta` was a
        // `BuiltinFunction` returning a bare `Type` with no real metaclass
        // behavior (`__instancecheck__`/`__subclasscheck__`/`.register()`
        // were unreachable, since `type(x) is abc.ABCMeta` was never true
        // and RustPython's generic custom-metaclass class-creation path
        // never ran for it).

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

        // __future__ is a real, complete, vendored Lib/__future__.py (a
        // proper `_Feature` class with `.compiler_flag`/`.optional`/
        // `.mandatory` attributes and working `getOptionalRelease()`/
        // `getMandatoryRelease()` methods) — no native registration needed,
        // and one used to shadow it here, representing each feature as a
        // bare 4-tuple instead. Real code that expects an actual `_Feature`
        // object (e.g. `Lib/codeop.py`'s `Compile.__call__`, which reads
        // `feature.compiler_flag` for every name in `all_feature_names`)
        // raised `AttributeError: 'tuple' object has no attribute
        // 'compiler_flag'` against the tuple stand-in.

        // Native asyncio module (basic event loop)
        modules.insert_str("asyncio", create_module("asyncio", create_asyncio_dict()));

        // Native atexit module (register/unregister exit callbacks)
        modules.insert_str("atexit", create_module("atexit", create_atexit_dict()));

        // Native contextvars module (ContextVar/Context/Token, real
        // per-Context isolation — see its own module doc comment).
        {
            let object_type = builtins
                .get(&interner::intern("object"))
                .cloned()
                .expect("object type must already be registered in builtins");
            modules.insert_str(
                "contextvars",
                create_module("contextvars", create_contextvars_dict(object_type)),
            );
        }

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

}
