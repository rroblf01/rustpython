use crate::bytecode::{needs_arg, CodeObject};
use crate::interner;
use crate::object::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

// Warnings recording for `warnings.catch_warnings(record=True)`. A stack of
// active record lists: `catch_warnings(record=True).__enter__` pushes a new
// list and returns it; native `warn()` appends WarningMessage objects to the
// innermost one; `__exit__` pops. `None` entries represent non-recording
// `catch_warnings` contexts (still tracked so a nested recording context
// restores the outer one correctly). This is what makes `unittest`'s
// `assertWarns`/`assertWarnsRegex` actually see warnings instead of failing
// with "... not triggered" (real trigger: `test_genericpath.py`'s
// `test_exists_bool`, which expects a `RuntimeWarning` when a bool is passed
// as a file descriptor).
thread_local! {
    static WARN_RECORD_STACK: std::cell::RefCell<Vec<Option<PyObjectRef>>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

thread_local! {
    static WARN_FILTER: std::cell::RefCell<String> = std::cell::RefCell::new("default".to_string());
}

thread_local! {
    static WARN_FILTER_STACK: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

thread_local! {
    static WARN_AS_ERROR: std::cell::RefCell<Option<(String, String)>> = const { std::cell::RefCell::new(None) };
}

fn warning_message_obj(message: PyObjectRef) -> PyObjectRef {
    let mut dict = AttrMap::new();
    dict.insert_str("message", message);
    dict.insert_str("category", py_none());
    dict.insert_str("filename", py_str(""));
    dict.insert_str("lineno", py_int(0));
    PyObjectRef::new(PyObject::Instance {
        typ: PyObjectRef::new(PyObject::Module {
            name: "warnings".to_string(),
            dict: Box::new(str_map_to_typedict(HashMap::new())),
        }),
        dict,
    })
}

pub fn warnings_emit(msg: &str, category: &str) {
    let filter = WARN_FILTER.with(|f| f.borrow().clone());
    if filter == "ignore" {
        return;
    }
    if filter == "error" {
        WARN_AS_ERROR.with(|c| {
            *c.borrow_mut() = Some((msg.to_string(), category.to_string()));
        });
        return;
    }
    let mut recorded = false;
    WARN_RECORD_STACK.with(|s| {
        if let Some(Some(list)) = s.borrow().last() {
            if let PyObject::List(items) = &mut *list.borrow_mut() {
                let cat_obj = {
                    let mut found: Option<PyObjectRef> = None;
                    let result = std::panic::catch_unwind(|| {
                        crate::vm::get_shared_builtins_module()
                    });
                    if let Ok(bmod) = result {
                        if let PyObject::Module { dict, .. } = &*bmod.borrow() {
                            if let Some(obj) = dict.get_str(category) {
                                found = Some(obj.clone());
                            }
                        }
                    }
                    found.unwrap_or_else(|| {
                        PyObjectRef::new(PyObject::Type {
                            name: category.to_string(),
                            dict: Box::new(str_map_to_typedict(HashMap::new())),
                            bases: vec![],
                            mro: vec![],
                        })
                    })
                };
                let mut dict = AttrMap::new();
                // Create message as an Exception with `typ` being the warning category
                // and `__class__` set to the actual category type (e.g., SyntaxWarning
                // from builtins) so that `issubclass(message.__class__, SyntaxWarning)`
                // is True via identity and `str(message)` is the warning text.
                let mut msg_obj = PyObjectRef::new(PyObject::Exception {
                    typ: category.to_string(),
                    args: vec![py_str(msg)],
                    cause: None,
                    suppress_context: false,
                    context: None,
                    traceback: None,
                    extra: None,
                });
                let _ = msg_obj.borrow_mut().set_attribute("__class__", cat_obj.clone());
                dict.insert_str("message", msg_obj);
                dict.insert_str("category", cat_obj.clone());
                dict.insert_str("filename", py_str("<input>"));
                dict.insert_str("lineno", py_int(1));
                dict.insert_str("line", py_str(""));
                let warn_type = PyObjectRef::new(PyObject::Type {
                    name: "WarningMessage".to_string(),
                    dict: Box::new(str_map_to_typedict(HashMap::new())),
                    bases: vec![],
                    mro: vec![],
                });
                let warn_obj = PyObjectRef::new(PyObject::Instance {
                    typ: warn_type,
                    dict,
                });
                items.push(warn_obj);
                recorded = true;
            }
        }
    });
    if !recorded {
        println!("Warning: {}", msg);
    }
}

pub fn warning_should_error() -> Option<(String, String)> {
    WARN_AS_ERROR.with(|c| c.borrow_mut().take())
}

pub fn warning_is_error_mode() -> bool {
    WARN_FILTER.with(|f| f.borrow().as_str() == "error")
}

pub fn warnings_push_record(list: Option<PyObjectRef>) {
    WARN_RECORD_STACK.with(|s| s.borrow_mut().push(list));
    WARN_FILTER_STACK.with(|st| {
        let cur = WARN_FILTER.with(|f| f.borrow().clone());
        st.borrow_mut().push(cur);
    });
}

pub fn warnings_pop_record() {
    WARN_RECORD_STACK.with(|s| {
        s.borrow_mut().pop();
    });
    WARN_FILTER_STACK.with(|st| {
        if let Some(prev) = st.borrow_mut().pop() {
            WARN_FILTER.with(|f| *f.borrow_mut() = prev);
        }
    });
}




pub fn create_warnings_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! warn_func {
        ($name:expr, $func:expr) => {
            d.insert(
                $name.to_string(),
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: $name.to_string(),
                    func: $func,
                }),
            );
        };
    }
// Real, shared `warnings.filters` state — was previously a fixed,
    // disconnected empty list (`d.insert_str("filters", py_list(vec![]))`
    // below) with `filterwarnings`/`_get_filters` both no-ops, so nothing
    // ever actually got recorded. `Lib/test/support/__init__.py`'s
    // `ignore_deprecations_from`/`clear_ignored_deprecations` (used by
    // `test_support.py`'s own `setUpClass`/`tearDownClass`, which assert
    // the filter COUNT actually grows/shrinks by 2 around two
    // `filterwarnings` calls) need this to be real, mutable, and shared
    // between `filterwarnings` (which appends) and `_get_filters`/
    // `filters` (which must read back the SAME list, not a fresh copy).
    // A `PyObjectRef::new(PyObject::List(...))` is `Rc<RefCell<_>>`-backed,
    // so storing one clone here and returning further clones from both
    // accessors keeps them all pointing at the same underlying storage.
    thread_local! {
        static WARN_FILTERS_LIST: std::cell::RefCell<Option<PyObjectRef>> = const { std::cell::RefCell::new(None) };
    }
    fn get_warn_filters_list() -> PyObjectRef {
        WARN_FILTERS_LIST.with(|f| {
            let mut opt = f.borrow_mut();
            if opt.is_none() {
                *opt = Some(py_list(vec![]));
            }
            opt.clone().unwrap()
        })
    }

    warn_func!("warn", |args| {
        let msg = if !args.is_empty() {
            args[0].str()
        } else {
            String::new()
        };
        let category = if args.len() > 1 {
            let cat_obj = &args[1];
            let borrowed = cat_obj.borrow();
            match &*borrowed {
                PyObject::BuiltinFunction { name, .. } => name.clone(),
                PyObject::Type { name, .. } => name.clone(),
                PyObject::Exception { typ, .. } => typ.clone(),
                _ => borrowed.type_name().to_string(),
            }
        } else {
            "UserWarning".to_string()
        };
        warnings_emit(&msg, &category);
        Ok(py_none())
    });

    warn_func!("simplefilter", |args| {
        if !args.is_empty() {
            let action = args[0].str();
            WARN_FILTER.with(|f| *f.borrow_mut() = action);
        }
        Ok(py_none())
    });

    // Insert the current filter state as a readable attribute — the SAME
    // shared list `filterwarnings`/`_get_filters` read and mutate.
    d.insert_str("filters", get_warn_filters_list());

    warn_func!("resetwarnings", |_| {
        let filters = get_warn_filters_list();
        if let PyObject::List(items) = &mut *filters.borrow_mut() {
            items.clear();
        }
        Ok(py_none())
    });
    // `warnings._get_filters()` — real CPython's internal accessor for the
    // `filters` list (added when the C `_warnings` module took over
    // filtering state). Was missing entirely (`AttributeError`), breaking
    // `Lib/test/support/__init__.py`'s own `swap_attr`-based filter-
    // save/restore helper used by `test_support.py`'s `setUpClass`. Now
    // returns the real, shared filters list (see `get_warn_filters_list`).
    warn_func!("_get_filters", |_| Ok(get_warn_filters_list()));
    // `warnings.filterwarnings(action, message="", category=Warning,
    // module="", lineno=0, append=False)` — was a complete no-op. Real
    // semantics only needed here: append/insert a `(action, message,
    // category, module, lineno)` tuple into the shared `filters` list —
    // `Lib/test/support/__init__.py`'s `ignore_deprecations_from`/
    // `clear_ignored_deprecations` only ever read the list back to count
    // or filter entries, never act on the filtering itself (this
    // interpreter's `warn()` doesn't consult `filters` to decide
    // suppression at all).
    warn_func!("filterwarnings", |args| {
        let action = args
            .first()
            .map(|a| a.str())
            .unwrap_or_else(|| "default".to_string());
        let kwargs = args.last().and_then(|a| {
            if let PyObject::Dict(d) = &*a.borrow() {
                Some((**d).clone())
            } else {
                None
            }
        });
        let get_kw = |name: &str| -> PyObjectRef {
            kwargs
                .as_ref()
                .and_then(|d| d.get(&py_str(name)).ok().flatten())
                .unwrap_or_else(py_none)
        };
        let message = {
            let m = get_kw("message");
            if matches!(&*m.borrow(), PyObject::None) {
                py_str("")
            } else {
                m
            }
        };
        let category = get_kw("category");
        let module = {
            let m = get_kw("module");
            if matches!(&*m.borrow(), PyObject::None) {
                py_str("")
            } else {
                m
            }
        };
        let lineno = {
            let l = get_kw("lineno");
            if matches!(&*l.borrow(), PyObject::None) {
                py_int(0)
            } else {
                l
            }
        };
        let append = get_kw("append").truthy();
        let entry = py_tuple(vec![py_str(&action), message, category, module, lineno]);
        let filters = get_warn_filters_list();
        if let PyObject::List(items) = &mut *filters.borrow_mut() {
            if append {
                items.push(entry);
            } else {
                items.insert(0, entry);
            }
        }
        Ok(py_none())
    });
    // Real CPython's `warnings.py` does `from _warnings import (..., _deprecated,
    // ...)` — a native-extension-only helper `nturl2path`/other stdlib
    // modules call directly (`warnings._deprecated("urllib.request...",
    // remove=(3, 15))`) to emit a standard-shaped `DeprecationWarning`.
    // Added as a thin wrapper around `warn` (this interpreter's own `warn`
    // just prints, so the exact message shape matters less than not
    // raising `AttributeError` on import).
    d.insert_str("_DEPRECATED_MSG", py_str("{name!r} is deprecated"));
    warn_func!("_deprecated", |args| {
        let name = if !args.is_empty() {
            args[0].str()
        } else {
            String::new()
        };
        println!("DeprecationWarning: {} is deprecated", name);
        Ok(py_none())
    });

    // catch_warnings() — a context manager real code uses to isolate/mute
    // warning state for a block (real trigger: CPython 3.14's own
    // `unittest/runner.py`, `with warnings.catch_warnings(): ...` around
    // each test run). Simplified: `record=True` returns a (permanently
    // empty, since `warn()` here just prints rather than recording)
    // list — good enough for code that only checks "were there 0 warnings"
    // or iterates expecting none, not for code asserting on captured
    // messages.
    let mut cw_dict = HashMap::new();
    cw_dict.insert_str("__init__", PyObjectRef::new(PyObject::BuiltinFunction {
        name: "__init__".to_string(),
        func: |args| {
            if let PyObject::Instance { dict, .. } = &mut *args[0].borrow_mut() {
                let record = args.iter().skip(1).any(|a| matches!(&*a.borrow(), PyObject::Dict(d) if d.get(&py_str("record")).ok().flatten().map(|v| v.truthy()).unwrap_or(false)));
                dict.insert_str("_record", py_bool(record));
            }
            Ok(py_none())
        },
    }));
    cw_dict.insert_str(
        "__enter__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__enter__".to_string(),
            func: |args| {
                let record = if let PyObject::Instance { dict, .. } = &*args[0].borrow() {
                    dict.get_str("_record").map(|v| v.truthy()).unwrap_or(false)
                } else {
                    false
                };
                if record {
                    let list = py_list(vec![]);
                    warnings_push_record(Some(list.clone()));
                    Ok(list)
                } else {
                    warnings_push_record(None);
                    Ok(py_none())
                }
            },
        }),
    );
    cw_dict.insert_str(
        "__exit__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__exit__".to_string(),
            func: |_args| {
                warnings_pop_record();
                Ok(py_bool(false))
            },
        }),
    );
    d.insert_str(
        "catch_warnings",
        PyObjectRef::new(PyObject::Type {
            name: "catch_warnings".to_string(),
            dict: Box::new(str_map_to_typedict(cw_dict)),
            bases: vec![],
            mro: vec![],
        }),
    );

    d
}


pub fn create_warnings_c_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! warn_func {
        ($name:expr, $func:expr) => {
            d.insert(
                $name.to_string(),
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: $name.to_string(),
                    func: $func,
                }),
            );
        };
    }
    warn_func!("warn", |args| {
        let msg = if !args.is_empty() {
            args[0].str()
        } else {
            String::new()
        };
        eprintln!("Warning: {}", msg);
        Ok(py_none())
    });
    d
}


pub fn create_abc_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! abc_func {
        ($name:expr, $func:expr) => {
            d.insert(
                $name.to_string(),
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: $name.to_string(),
                    func: $func,
                }),
            );
        };
    }

    // `abc.get_cache_token()` — real CPython returns an opaque token that
    // changes whenever the internal ABC registry cache is invalidated
    // (callers compare it across calls to detect staleness). Missing
    // entirely here (`AttributeError: 'module' object has no attribute
    // 'get_cache_token'` — real trigger: CPython's own `test_abc.py`, and
    // by extension `_py_abc`, which is aliased to this same dict). This
    // implementation doesn't track cache invalidation at all, so a
    // constant token is a safe, conservative stand-in — "never stale" is
    // the correct simplification when there's no cache to invalidate.
    abc_func!("get_cache_token", |_args| Ok(py_int(0)));

    // ABC class — a REAL Type that can be used as a base class.
    // Previously was a BuiltinFunction returning an Instance, which meant
    // `class C(abc.ABC): ...` had a non-Type as its base, breaking the
    // entire inheritance chain and preventing __abstractmethods__ from ever
    // being computed.
    {
        let abc_type = PyObjectRef::new(PyObject::Type {
            name: "ABC".to_string(),
            dict: Box::new(str_map_to_typedict(HashMap::new())),
            bases: vec![],
            mro: vec![],
        });
        d.insert_str("ABC", abc_type);
    }

    // abstractmethod — marks the function with `__isabstractmethod__ =
    // True` (previously just returned it unchanged, with no marker at
    // all — meant `update_abstractmethods`/`isabstract`/anything checking
    // `getattr(f, '__isabstractmethod__', False)` could never find a
    // single abstract method anywhere, silently defeating the entire ABC
    // mechanism) and returns it, matching real CPython.
    abc_func!("abstractmethod", |args| {
        if args.is_empty() {
            return Ok(py_none());
        }
        let f = &args[0];
        let _ = f
            .borrow_mut()
            .set_attribute("__isabstractmethod__", py_bool(true));
        Ok(f.clone())
    });

    // abstractclassmethod — deprecated alias for classmethod + abstractmethod.
    abc_func!("abstractclassmethod", |args| {
        let f = &args[0];
        let _ = f
            .borrow_mut()
            .set_attribute("__isabstractmethod__", py_bool(true));
        let cm_fn = crate::object::builtin_classmethod as crate::object::BuiltinFunc;
        let cm = PyObjectRef::new(PyObject::BuiltinFunction {
            name: "classmethod".to_string(),
            func: cm_fn,
        });
        crate::object::with_vm_mut(|vm| vm.call_function(cm, vec![f.clone()], vec![]))?
    });

    // abstractstaticmethod — deprecated alias for staticmethod + abstractmethod.
    abc_func!("abstractstaticmethod", |args| {
        let f = &args[0];
        let _ = f
            .borrow_mut()
            .set_attribute("__isabstractmethod__", py_bool(true));
        let sm_fn = crate::object::builtin_staticmethod as crate::object::BuiltinFunc;
        let sm = PyObjectRef::new(PyObject::BuiltinFunction {
            name: "staticmethod".to_string(),
            func: sm_fn,
        });
        crate::object::with_vm_mut(|vm| vm.call_function(sm, vec![f.clone()], vec![]))?
    });

    // abstractproperty — deprecated alias for property + abstractmethod.
    // Marks the getter/setter/deleter with __isabstractmethod__, creates
    // the property, AND sets __isabstractmethod__ on the property itself
    // so update_abstractmethods can detect it during class creation.
    abc_func!("abstractproperty", |args| {
        // First mark all function arguments as abstract
        for arg in args.iter() {
            let _ = arg.borrow_mut().set_attribute("__isabstractmethod__", py_bool(true));
        }
        // Create the property
        let prop_fn = crate::object::builtin_property as crate::object::BuiltinFunc;
        let prop_ctor = PyObjectRef::new(PyObject::BuiltinFunction {
            name: "property".to_string(),
            func: prop_fn,
        });
        let result = crate::object::with_vm_mut(|vm| {
            vm.call_function(prop_ctor, args.to_vec(), vec![])
        })??;
        // Mark the property object itself as abstract
        let _ = result.borrow_mut().set_attribute("__isabstractmethod__", py_bool(true));
        Ok(result)
    });

    // update_abstractmethods(cls) — recomputes `cls.__abstractmethods__`
    // from scratch: parent classes' own abstract methods that `cls`
    // still hasn't overridden, plus any of `cls`'s OWN methods newly
    // marked via `@abstractmethod`. Missing entirely before (`AttributeError:
    // 'module' object has no attribute 'update_abstractmethods'` — real
    // trigger: `numbers.py`'s own test suite calling this directly after
    // patching in concrete method implementations).
    abc_func!("update_abstractmethods", |args| {
        if args.is_empty() {
            return Err(PyError::type_error(
                "update_abstractmethods() requires 1 argument",
            ));
        }
        let cls = args[0].clone();
    // Always scan for abstract methods — don't skip even if cls
    // doesn't have __abstractmethods__ yet. CPython's ABCMeta.__new__
    // always calls this after class creation.
    {}
        let mut abstracts: Vec<String> = Vec::new();
        let extracted: Option<(Vec<PyObjectRef>, Vec<String>)> =
            if let PyObject::Type { dict, bases, .. } = &*cls.borrow() {
                Some((
                    bases.clone(),
                    dict.keys()
                        .map(|k| interner::lookup_str(*k).to_string())
                        .collect(),
                ))
            } else {
                None
            };
        let (bases, dict_names) = match extracted {
            Some(v) => v,
            None => return Ok(cls),
        };
        for base in &bases {
            if let Ok(base_abstracts) = base.borrow().get_attribute("__abstractmethods__") {
                if let PyObject::FrozenSet(items) = &*base_abstracts.borrow() {
                    for item in items.to_vec() {
                        let name = item.str();
                        let still_abstract = if let Ok(val) = cls.borrow().get_attribute(&name) {
                            val.borrow()
                                .get_attribute("__isabstractmethod__")
                                .map(|v| v.truthy())
                                .unwrap_or(false)
                        } else {
                            true
                        };
                        if still_abstract && !abstracts.contains(&name) {
                            abstracts.push(name);
                        }
                    }
                }
            }
        }
        for name in &dict_names {
            if let Ok(val) = cls.borrow().get_attribute(name) {
                if val
                    .borrow()
                    .get_attribute("__isabstractmethod__")
                    .map(|v| v.truthy())
                    .unwrap_or(false)
                    && !abstracts.contains(name)
                {
                    abstracts.push(name.clone());
                }
            }
        }
        let mut set = PySet::new();
        for name in &abstracts {
            set.add(py_str(name))?;
        }
        cls.borrow_mut().set_attribute(
            "__abstractmethods__",
            PyObjectRef::imm(PyObject::FrozenSet(set)),
        )?;
        Ok(cls)
    });

    // ABCMeta — minimal metaclass stub (needed by io.py et al)
    abc_func!("ABCMeta", |args| {
        // In CPython, ABCMeta(name, bases, namespace) -> new class
        // For our stub, return a Type object with the given name
        let name = if !args.is_empty() {
            args[0].str()
        } else {
            "ABCMeta".to_string()
        };
        Ok(PyObjectRef::new(PyObject::Type {
            name,
            dict: Box::new(str_map_to_typedict(HashMap::new())),
            bases: vec![],
            mro: vec![],
        }))
    });

    d
}


pub fn create_dataclasses_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! dc_func {
        ($name:expr, $func:expr) => {
            d.insert(
                $name.to_string(),
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: $name.to_string(),
                    func: $func,
                }),
            );
        };
    }

    // dataclass(cls) — decorator that marks cls with _dataclass_ attr.
    // NOTE: still just a marker, not real dataclass semantics — a genuine
    // @dataclass should synthesize __init__/__repr__/__eq__/__hash__ from
    // the class's own annotated fields (respecting field()'s
    // default/default_factory/kw_only/etc.), which this does not do at all.
    // That's a separately-tracked, larger gap; this function only fixes the
    // narrower, previously-crashing case of the PARAMETERIZED decorator
    // form (`@dataclass(frozen=True)`, real code — e.g. CPython's own
    // `_colorize.py` theme classes): called with kwargs only (no `cls`
    // positional argument, since our calling convention packs kwargs as a
    // trailing dict arg), this used to just return that kwargs dict itself,
    // which the decorator machinery then tried to CALL as if it were the
    // real decorator — "'dict' object is not callable". Detect that shape
    // and return a real closure instead.
    fn is_class_or_instance(v: &PyObjectRef) -> bool {
        matches!(
            &*v.borrow(),
            PyObject::Type { .. } | PyObject::Instance { .. }
        )
    }
    fn mark_dataclass(cls: &PyObjectRef) -> PyResult<PyObjectRef> {
        let mut borrowed = cls.borrow_mut();
        if let PyObject::Instance { ref mut dict, .. } = &mut *borrowed {
            dict.insert_str("_dataclass_", py_bool(true));
        }
        if let PyObject::Type { ref mut dict, .. } = &mut *borrowed {
            dict.insert_str("_dataclass_", py_bool(true));
        }
        drop(borrowed);
        Ok(cls.clone())
    }
    dc_func!("dataclass", |args| {
        if args.is_empty() || !is_class_or_instance(&args[0]) {
            // Parameterized form: @dataclass(frozen=True, ...) — return a
            // decorator closure that applies the same (still-stub) marking
            // once the real class is passed to it.
            return Ok(PyObjectRef::imm(PyObject::Closure(std::rc::Rc::new(
                |inner_args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                    if inner_args.is_empty() {
                        return Err(PyError::type_error(
                            "dataclass() decorator missing required argument (cls)",
                        ));
                    }
                    mark_dataclass(&inner_args[0])
                },
            ))));
        }
        mark_dataclass(&args[0])
    });

    // field() — returns empty dict as a field descriptor
    dc_func!("field", |_| { Ok(py_dict()) });

    // Field — the class dataclasses.field()/introspection would normally
    // return/expose (real attrs: name, type, default, default_factory,
    // repr, hash, init, compare, metadata, kw_only). This bare marker class
    // exists mainly so `from dataclasses import Field` succeeds at all
    // (needed transitively by real CPython stdlib source we vendor
    // verbatim, e.g. `_colorize.py`'s own type annotations) — actual
    // dataclass field generation is still a stub (see the `dataclass()`
    // function above: real @dataclass should synthesize __init__/__repr__/
    // __eq__ from annotated fields; this only tags the class, a known,
    // separately-tracked gap, not something this placeholder fixes.
    d.insert_str(
        "Field",
        PyObjectRef::new(PyObject::Type {
            name: "Field".to_string(),
            dict: Box::new(str_map_to_typedict(HashMap::new())),
            bases: vec![],
            mro: vec![],
        }),
    );

    // asdict(obj) — shallow dict copy
    dc_func!("asdict", |args| {
        if args.is_empty() {
            return Err(PyError::type_error(
                "asdict() missing required argument (obj)",
            ));
        }
        let obj = &args[0];
        let borrowed = obj.borrow();
        match &*borrowed {
            PyObject::Instance { dict, .. } => {
                let mut new_dict = PyDict::new();
                for (k, v) in dict.iter() {
                    let _ = new_dict.set(py_str(k), v.clone());
                }
                Ok(PyObjectRef::new(PyObject::Dict(Box::new(new_dict))))
            }
            PyObject::Dict(pydict) => {
                let mut new_dict = PyDict::new();
                for (k, v) in pydict.items() {
                    let _ = new_dict.set(k, v);
                }
                Ok(PyObjectRef::new(PyObject::Dict(Box::new(new_dict))))
            }
            _ => Err(PyError::type_error(
                "asdict() argument must be a dataclass instance",
            )),
        }
    });

    // astuple(obj) — shallow tuple copy
    dc_func!("astuple", |args| {
        if args.is_empty() {
            return Err(PyError::type_error(
                "astuple() missing required argument (obj)",
            ));
        }
        let obj = &args[0];
        let borrowed = obj.borrow();
        match &*borrowed {
            PyObject::Instance { dict, .. } => {
                let items: Vec<PyObjectRef> = dict.values().cloned().collect();
                Ok(PyObjectRef::imm(PyObject::Tuple(items)))
            }
            PyObject::Dict(pydict) => {
                let items: Vec<PyObjectRef> = pydict.values();
                Ok(PyObjectRef::imm(PyObject::Tuple(items)))
            }
            _ => Err(PyError::type_error(
                "astuple() argument must be a dataclass instance",
            )),
        }
    });

    // is_dataclass(obj) — checks for _dataclass_ attribute
    dc_func!("is_dataclass", |args| {
        if args.is_empty() {
            return Err(PyError::type_error(
                "is_dataclass() missing required argument (obj)",
            ));
        }
        let obj = &args[0];
        let borrowed = obj.borrow();
        match &*borrowed {
            PyObject::Instance { dict, .. } => Ok(py_bool(dict.contains_key_str("_dataclass_"))),
            PyObject::Type { dict, .. } => Ok(py_bool(dict.contains_key_str("_dataclass_"))),
            PyObject::Dict(pydict) => {
                let _ = pydict;
                Ok(py_bool(false))
            }
            _ => Ok(py_bool(false)),
        }
    });

    // make_dataclass(name, fields) — simple Type object
    dc_func!("make_dataclass", |args| {
        if args.len() < 2 {
            return Err(PyError::type_error(
                "make_dataclass() requires at least 2 arguments (name, fields)",
            ));
        }
        let name = args[0].str();
        Ok(PyObjectRef::new(PyObject::Type {
            name,
            dict: Box::new(str_map_to_typedict(HashMap::new())),
            bases: vec![],
            mro: vec![],
        }))
    });

    d
}


pub fn create_unittest_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();

    macro_rules! unittest_func {
        ($name:expr, $func:expr) => {
            d.insert(
                $name.to_string(),
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: $name.to_string(),
                    func: $func,
                }),
            );
        };
    }

    // Create the TestCase class
    let mut tc_dict = HashMap::new();

    // __init__ — no-op stub
    tc_dict.insert_str(
        "__init__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__init__".to_string(),
            func: |_args| Ok(py_none()),
        }),
    );

    // assertEqual(self, a, b) — no-op stub
    tc_dict.insert_str(
        "assertEqual",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "assertEqual".to_string(),
            func: |_args| Ok(py_none()),
        }),
    );

    // assertTrue(self, expr) — no-op stub
    tc_dict.insert_str(
        "assertTrue",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "assertTrue".to_string(),
            func: |_args| Ok(py_none()),
        }),
    );

    // assertFalse(self, expr) — no-op stub
    tc_dict.insert_str(
        "assertFalse",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "assertFalse".to_string(),
            func: |_args| Ok(py_none()),
        }),
    );

    // assertRaises(self, exc, callable=None, *args) — no-op stub
    tc_dict.insert_str(
        "assertRaises",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "assertRaises".to_string(),
            func: |_args| Ok(py_none()),
        }),
    );

    // assertIn(self, a, b) — no-op stub
    tc_dict.insert_str(
        "assertIn",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "assertIn".to_string(),
            func: |_args| Ok(py_none()),
        }),
    );

    // assertNotIn(self, a, b) — no-op stub
    tc_dict.insert_str(
        "assertNotIn",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "assertNotIn".to_string(),
            func: |_args| Ok(py_none()),
        }),
    );

    // assertIsNone(self, obj) — no-op stub
    tc_dict.insert_str(
        "assertIsNone",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "assertIsNone".to_string(),
            func: |_args| Ok(py_none()),
        }),
    );

    // assertIsNotNone(self, obj) — no-op stub
    tc_dict.insert_str(
        "assertIsNotNone",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "assertIsNotNone".to_string(),
            func: |_args| Ok(py_none()),
        }),
    );

    let testcase_class = PyObjectRef::new(PyObject::Type {
        name: "TestCase".to_string(),
        dict: Box::new(str_map_to_typedict(tc_dict)),
        bases: vec![],
        mro: vec![],
    });

    d.insert_str("TestCase", testcase_class);

    // main() — stub that does nothing
    unittest_func!("main", |_args| { Ok(py_none()) });

    // expectedFailure decorator stub — returns the function unchanged
    unittest_func!("expectedFailure", |args| {
        if args.is_empty() {
            return Ok(py_none());
        }
        Ok(args[0].clone())
    });

    // skip decorator stub — returns the function unchanged
    unittest_func!("skip", |args| {
        if args.is_empty() {
            return Ok(py_none());
        }
        Ok(args[0].clone())
    });

    d
}


pub fn create_zipimport_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! zip_func {
        ($name:expr, $func:expr) => {
            d.insert(
                $name.to_string(),
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: $name.to_string(),
                    func: $func,
                }),
            );
        };
    }
    zip_func!("zipimporter", |args| {
        let _path = if !args.is_empty() {
            args[0].str()
        } else {
            String::new()
        };
        let mut inst_dict = AttrMap::new();
        inst_dict.insert_str(
            "find_spec",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "find_spec".to_string(),
                func: |_| Ok(py_none()),
            }),
        );
        inst_dict.insert_str(
            "find_module",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "find_module".to_string(),
                func: |_| Ok(py_none()),
            }),
        );
        inst_dict.insert_str(
            "get_code",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "get_code".to_string(),
                func: |_| Ok(py_none()),
            }),
        );
        inst_dict.insert_str(
            "get_source",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "get_source".to_string(),
                func: |_| Ok(py_none()),
            }),
        );
        Ok(PyObjectRef::new(PyObject::Instance {
            typ: py_str("zipimporter"),
            dict: inst_dict,
        }))
    });
    d.insert_str("_zip_directory_cache", py_dict());
    d
}

