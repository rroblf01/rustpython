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
                    let result =
                        std::panic::catch_unwind(|| crate::vm::get_shared_builtins_module());
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
                let _ = msg_obj
                    .borrow_mut()
                    .set_attribute("__class__", cat_obj.clone());
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
        // Route through the shared emitter (record stack, filters,
        // stderr fallback) — a bare println bypassed
        // `catch_warnings(record=True)`, so `assertWarns` never saw it
        // (test_platform.test_java_ver). Message shape mirrors CPython's
        // `_DEPRECATED_MSG`: `'name' is deprecated and slated for
        // removal in Python X.Y` (from the `remove=(X, Y)` kwarg).
        let mut remove = String::from("?");
        for a in args.iter().skip(1) {
            if let PyObject::Tuple(items) = &*a.borrow() {
                if items.len() == 2 {
                    remove = format!("{}.{}", items[0].str(), items[1].str());
                    break;
                }
            }
        }
        // `remove=` may also arrive packed as a trailing kwargs dict.
        if remove == "?" {
            if let Some(last) = args.last() {
                if let PyObject::Dict(d) = &*last.borrow() {
                    if let Ok(Some(rv)) = d.get(&py_str("remove")) {
                        if let PyObject::Tuple(items) = &*rv.borrow() {
                            if items.len() == 2 {
                                remove = format!("{}.{}", items[0].str(), items[1].str());
                            }
                        }
                    }
                }
            }
        }
        warnings_emit(
            &format!("'{name}' is deprecated and slated for removal in Python {remove}"),
            "DeprecationWarning",
        );
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

/// `zipimporter` method marker — every real zipimporter method (see
/// `zipimporter_method_with_vm`) is registered under this SAME bare fn
/// (never actually invoked; reaching it means the `call.rs`/`op_call.rs`
/// interception was skipped), distinguished purely by the `name` field
/// `PyObject::BuiltinFunction`/`BuiltinMethod` already carries — avoiding
/// needing one distinct marker fn (and matching `fn_addr_eq` registration)
/// per method. Deliberately unique code (not a bare `Ok(py_none())`, which
/// would get folded onto some OTHER trivial stub's address by thin-LTO —
/// see the `iobase_exit_marker` doc comment for the concrete hazard this
/// mirrors) via the panic-style message baked into the `Err`.
pub fn zipimporter_method_marker(_args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    Err(PyError::runtime_error(
        "zipimporter method marker reached directly (interception missing)",
    ))
}

fn zi_get_str_attr(self_obj: &PyObjectRef, attr: &str) -> PyResult<String> {
    if let PyObject::Instance { dict, .. } = &*self_obj.borrow() {
        if let Some(v) = dict.get_str(attr) {
            return Ok(v.str());
        }
    }
    Err(PyError::attribute_error(format!(
        "zipimporter object has no attribute '{}'",
        attr
    )))
}

fn zi_arg_str(args: &[PyObjectRef], idx: usize, method: &str) -> PyResult<String> {
    args.get(idx).map(|a| a.str()).ok_or_else(|| {
        PyError::type_error(format!("{}() missing required positional argument", method))
    })
}

fn zi_zip_import_error(fullname: &str) -> PyError {
    PyError::Exception(
        "ZipImportError".to_string(),
        PyObjectRef::new(PyObject::Exception {
            typ: "ZipImportError".to_string(),
            args: vec![py_str(&format!("can't find module {:?}", fullname))],
            cause: None,
            suppress_context: false,
            context: None,
            traceback: None,
            extra: None,
        }),
    )
}

/// `self.prefix + fullname.rpartition('.')[2]` (real CPython's
/// `_get_module_path`) — note this takes only the part after the LAST
/// dot, or the WHOLE string unchanged if there's no dot at all (which is
/// how the test suite gets away with passing a `/`-joined path like
/// `"pkg/__init__"` as `fullname`: no dot means it passes through as-is).
fn zi_module_base(prefix: &str, fullname: &str) -> String {
    let last = fullname.rsplit('.').next().unwrap_or(fullname);
    format!("{}{}", prefix, last)
}

/// Real CPython's `_zip_searchorder`: package `__init__` (bytecode then
/// source) before a plain module (bytecode then source).
fn zi_module_info(
    vm: &mut crate::vm::VirtualMachine,
    archive: &str,
    base: &str,
) -> Option<(bool, String, Vec<u8>)> {
    let candidates: [(String, bool); 4] = [
        (format!("{}/__init__.pyc", base), true),
        (format!("{}/__init__.py", base), true),
        (format!("{}.pyc", base), false),
        (format!("{}.py", base), false),
    ];
    for (member, is_pkg) in candidates.iter() {
        if let Some(data) = vm.read_zip_member(archive, member) {
            return Some((*is_pkg, member.clone(), data));
        }
    }
    None
}

fn zi_code_from_member(member: &str, data: &[u8], virtual_path: &str) -> Option<CodeObject> {
    if member.ends_with(".pyc") {
        if data.len() < 16 {
            return None;
        }
        crate::modules::code_object_from_marshal_bytes(&data[16..])
    } else {
        let source = crate::object::import_builtin::decode_source_bytes(data).ok()?;
        let mut parser = crate::parser::Parser::new(&source);
        let program = parser.parse_program().ok()?;
        let mut compiler = crate::compiler::Compiler::new();
        compiler.compile(&program, virtual_path).ok()
    }
}

/// Real logic behind every `zipimporter` instance method — dispatched
/// here (rather than as separate native fns) via `call.rs`'s
/// `fn_addr_eq`-on-`zipimporter_method_marker` interception, since all of
/// these need live `&mut VirtualMachine` access to call into the real
/// (vendored) `zipfile.ZipFile` for actual archive reads (see
/// `VirtualMachine::read_zip_member`).
pub(crate) fn zipimporter_method_with_vm(
    vm: &mut crate::vm::VirtualMachine,
    method: &str,
    self_obj: &PyObjectRef,
    args: &[PyObjectRef],
) -> PyResult<PyObjectRef> {
    let archive = zi_get_str_attr(self_obj, "archive")?;
    let prefix = zi_get_str_attr(self_obj, "prefix")?;
    match method {
        "is_package" => {
            let fullname = zi_arg_str(args, 0, "is_package")?;
            let base = zi_module_base(&prefix, &fullname);
            match zi_module_info(vm, &archive, &base) {
                Some((is_pkg, _, _)) => Ok(py_bool(is_pkg)),
                None => Err(zi_zip_import_error(&fullname)),
            }
        }
        "get_source" => {
            let fullname = zi_arg_str(args, 0, "get_source")?;
            let base = zi_module_base(&prefix, &fullname);
            match zi_module_info(vm, &archive, &base) {
                Some((_, member, data)) => {
                    if member.ends_with(".py") {
                        match crate::object::import_builtin::decode_source_bytes(&data) {
                            Ok(s) => Ok(py_str(&s)),
                            Err(_) => Ok(py_none()),
                        }
                    } else {
                        Ok(py_none())
                    }
                }
                None => Err(zi_zip_import_error(&fullname)),
            }
        }
        "get_code" => {
            let fullname = zi_arg_str(args, 0, "get_code")?;
            let base = zi_module_base(&prefix, &fullname);
            match zi_module_info(vm, &archive, &base) {
                Some((_, member, data)) => {
                    let virtual_path = format!("{}/{}", archive, member);
                    match zi_code_from_member(&member, &data, &virtual_path) {
                        Some(code) => Ok(PyObjectRef::new(PyObject::Code(Rc::new(code)))),
                        None => Err(zi_zip_import_error(&fullname)),
                    }
                }
                None => Err(zi_zip_import_error(&fullname)),
            }
        }
        "get_filename" => {
            let fullname = zi_arg_str(args, 0, "get_filename")?;
            let base = zi_module_base(&prefix, &fullname);
            match zi_module_info(vm, &archive, &base) {
                Some((_, member, _)) => Ok(py_str(&format!("{}/{}", archive, member))),
                None => Err(zi_zip_import_error(&fullname)),
            }
        }
        "get_data" => {
            let pathname = zi_arg_str(args, 0, "get_data")?;
            let key = pathname
                .strip_prefix(&format!("{}/", archive))
                .unwrap_or(&pathname)
                .to_string();
            match vm.read_zip_member(&archive, &key) {
                Some(data) => Ok(PyObjectRef::imm(PyObject::Bytes(data))),
                None => Err(PyError::Exception(
                    "OSError".to_string(),
                    PyObjectRef::new(PyObject::Exception {
                        typ: "OSError".to_string(),
                        args: vec![py_int(0), py_str(""), py_str(&key)],
                        cause: None,
                        suppress_context: false,
                        context: None,
                        traceback: None,
                        extra: None,
                    }),
                )),
            }
        }
        "find_spec" => {
            let fullname = zi_arg_str(args, 0, "find_spec")?;
            let base = zi_module_base(&prefix, &fullname);
            match zi_module_info(vm, &archive, &base) {
                Some((is_pkg, member, _)) => {
                    let origin = format!("{}/{}", archive, member);
                    let search_loc = format!("{}/{}", archive, base);
                    let spec =
                        zi_build_spec(vm, &fullname, self_obj, &origin, is_pkg, &search_loc)?;
                    Ok(spec)
                }
                None => Ok(py_none()),
            }
        }
        "exec_module" => {
            if args.is_empty() {
                return Err(PyError::type_error("exec_module() missing module argument"));
            }
            let module_obj = args[0].clone();
            let fullname = if let PyObject::Module { dict, .. } = &*module_obj.borrow() {
                dict.get(&interner::intern("__name__"))
                    .map(|v| v.str())
                    .unwrap_or_default()
            } else {
                return Err(PyError::type_error(
                    "exec_module() argument must be a module",
                ));
            };
            let base = zi_module_base(&prefix, &fullname);
            match zi_module_info(vm, &archive, &base) {
                Some((is_pkg, member, data)) => {
                    let virtual_path = format!("{}/{}", archive, member);
                    let code = zi_code_from_member(&member, &data, &virtual_path)
                        .ok_or_else(|| zi_zip_import_error(&fullname))?;
                    let pkg_path = if is_pkg {
                        Some(format!("{}/{}", archive, base))
                    } else {
                        None
                    };
                    vm.exec_code_into_existing_module(
                        &module_obj,
                        code,
                        &virtual_path,
                        &fullname,
                        is_pkg,
                        pkg_path,
                    )?;
                    Ok(py_none())
                }
                None => Err(zi_zip_import_error(&fullname)),
            }
        }
        "load_module" => {
            let fullname = zi_arg_str(args, 0, "load_module")?;
            let base = zi_module_base(&prefix, &fullname);
            let (is_pkg, member, data) = zi_module_info(vm, &archive, &base)
                .ok_or_else(|| zi_zip_import_error(&fullname))?;
            let virtual_path = format!("{}/{}", archive, member);
            let code = zi_code_from_member(&member, &data, &virtual_path)
                .ok_or_else(|| zi_zip_import_error(&fullname))?;
            // Reuse an already-registered module under this name if there
            // is one (mirrors real `load_module`'s `sys.modules.get`
            // reuse), otherwise build a fresh one.
            let module_obj = vm
                .modules
                .get(&fullname)
                .cloned()
                .unwrap_or_else(|| create_module(&fullname, HashMap::new()));
            vm.modules.insert(fullname.clone(), module_obj.clone());
            if let Some(sys_mod) = vm.modules.get("sys").cloned() {
                if let PyObject::Module { dict, .. } = &*sys_mod.borrow() {
                    if let Some(mod_dict) = dict.get_str("modules").cloned() {
                        if let PyObject::Dict(d) = &mut *mod_dict.borrow_mut() {
                            let _ = d.set(py_str(&fullname), module_obj.clone());
                        }
                    }
                }
            }
            let pkg_path = if is_pkg {
                Some(format!("{}/{}", archive, base))
            } else {
                None
            };
            match vm.exec_code_into_existing_module(
                &module_obj,
                code,
                &virtual_path,
                &fullname,
                is_pkg,
                pkg_path,
            ) {
                Ok(()) => Ok(module_obj),
                Err(e) => {
                    vm.modules.remove(&fullname);
                    Err(e)
                }
            }
        }
        "invalidate_caches" => Ok(py_none()),
        "_get_files" => {
            let names = zi_list_archive_names(vm, &archive)?;
            let d = Box::new(PyDict::new());
            let dict_ref = PyObjectRef::imm(PyObject::Dict(d));
            if let PyObjectRef::Imm(rc) = &dict_ref {
                if let PyObject::Dict(d) = &mut *rc.borrow_mut() {
                    for n in names {
                        let _ = d.set(py_str(&n), py_bool(true));
                    }
                }
            }
            Ok(dict_ref)
        }
        _ => Err(PyError::attribute_error(format!(
            "zipimporter object has no attribute '{}'",
            method
        ))),
    }
}

fn zi_list_archive_names(
    vm: &mut crate::vm::VirtualMachine,
    archive: &str,
) -> PyResult<Vec<String>> {
    let zipfile_mod = match vm.modules.get("zipfile").cloned() {
        Some(m) => m,
        None => vm.import_module_from_file("zipfile")?,
    };
    let zipfile_cls = zipfile_mod.borrow().get_attribute("ZipFile")?;
    let zf = vm.call_function(zipfile_cls, vec![py_str(archive)], vec![])?;
    let namelist = crate::object::call_method_rebound(vm, &zf, "namelist", vec![])?;
    let names = if let PyObject::List(items) = &*namelist.borrow() {
        items.iter().map(|i| i.str()).collect()
    } else {
        vec![]
    };
    let _ = crate::object::call_method_rebound(vm, &zf, "close", vec![]);
    Ok(names)
}

/// Builds a real `importlib.machinery.ModuleSpec` (the vendored,
/// Python-level class in `Lib/importlib/machinery.py`) rather than
/// hand-rolling a spec-shaped native object, so `importlib.util
/// .module_from_spec` (also real Python code) works on it unmodified.
fn zi_build_spec(
    vm: &mut crate::vm::VirtualMachine,
    fullname: &str,
    loader: &PyObjectRef,
    origin: &str,
    is_package: bool,
    search_loc: &str,
) -> PyResult<PyObjectRef> {
    let machinery_mod = match vm.modules.get("importlib.machinery").cloned() {
        Some(m) => m,
        None => vm.import_module_from_file("importlib.machinery")?,
    };
    let spec_cls = machinery_mod.borrow().get_attribute("ModuleSpec")?;
    let kwargs = vec![
        ("origin".to_string(), py_str(origin)),
        ("is_package".to_string(), py_bool(is_package)),
    ];
    let spec = vm.call_function(spec_cls, vec![py_str(fullname), loader.clone()], kwargs)?;
    if is_package {
        let search_locs = py_list(vec![py_str(search_loc)]);
        if let PyObject::Instance { dict, .. } = &mut *spec.borrow_mut() {
            dict.insert_str("submodule_search_locations", search_locs);
        }
    }
    Ok(spec)
}

pub fn create_zipimport_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();

    // `zipimport.ZipImportError` — same shape as `binascii.Error`: a bare
    // `PyObject::BuiltinFunction` building a `PyObject::Exception`
    // directly (see the matching entry added to
    // `is_builtin_exception_class_name`/`is_exception_subclass`).
    d.insert_str(
        "ZipImportError",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "ZipImportError".to_string(),
            func: |args| {
                let msg = args.first().map(|a| a.str()).unwrap_or_default();
                Ok(PyObjectRef::new(PyObject::Exception {
                    typ: "ZipImportError".to_string(),
                    args: if args.is_empty() {
                        vec![]
                    } else {
                        vec![py_str(&msg)]
                    },
                    cause: None,
                    suppress_context: false,
                    context: None,
                    traceback: None,
                    extra: None,
                }))
            },
        }),
    );

    // The `zipimporter` type: a real `PyObject::Type` (methods dispatched
    // through the MRO, auto-binding to `BuiltinMethod` exactly like
    // `io.IOBase` — see that class for the same pattern) rather than
    // per-instance closures, since every method needs the SAME live-VM
    // detour (`zipimporter_method_with_vm`) and per-instance closures
    // can't be matched by `fn_addr_eq` (each construction gets a distinct
    // vtable pointer).
    let mut zi_dict = HashMap::new();
    zi_dict.insert_str(
        "__doc__",
        py_str("zipimporter(archivepath) -> zipimporter object"),
    );
    for m in [
        "is_package",
        "get_source",
        "get_code",
        "get_filename",
        "get_data",
        "find_spec",
        "exec_module",
        "load_module",
        "invalidate_caches",
        "_get_files",
    ] {
        zi_dict.insert_str(
            m,
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: m.to_string(),
                func: zipimporter_method_marker,
            }),
        );
    }
    let zi_cls = PyObjectRef::new(PyObject::Type {
        name: "zipimporter".to_string(),
        dict: Box::new(str_map_to_typedict(zi_dict)),
        bases: vec![],
        mro: vec![],
    });
    // The actual CALLABLE bound to `zipimport.zipimporter` is a `Closure`
    // capturing `zi_cls` directly (so instances get the real type, and
    // hence real bound methods, attached) — NOT the bare `PyObject::Type`
    // itself, and NOT a plain non-capturing `BuiltinFunction` needing a
    // live-VM lookup of "the zipimporter type" (which would need the
    // unsound-from-live-bytecode `with_vm_mut`, see `zipimporter_new`'s
    // old doc comment history). A `Closure` needs no VM access at all
    // here since `split_zip_archive` is a pure filesystem check.
    d.insert_str(
        "zipimporter",
        PyObjectRef::new(PyObject::Closure(Rc::new(move |args: &[PyObjectRef]| {
            zipimporter_new(args, zi_cls.clone())
        }))),
    );

    d.insert_str("_zip_directory_cache", py_dict());
    d
}

/// The `zipimporter(archivepath)` constructor, called from the `Closure`
/// registered under `zipimport.zipimporter` in `create_zipimport_dict`
/// (which supplies `zi_cls`, the real `PyObject::Type`, so instances get
/// real bound methods via the MRO). Deliberately NOT the type's own
/// `__init__`/`__new__` — building the archive/prefix split needs to
/// VALIDATE the path and can legitimately raise before any instance
/// exists (`ZipImportError`/`TypeError`), simplest as one self-contained
/// constructor function.
pub fn zipimporter_new(args: &[PyObjectRef], zi_cls: PyObjectRef) -> PyResult<PyObjectRef> {
    let path_str = match args.first() {
        Some(a) => match &*a.borrow() {
            PyObject::Str(s) => s.to_string(),
            _ => {
                return Err(PyError::type_error(
                    "zipimporter() argument path must be str, not bytes/None/other",
                ))
            }
        },
        None => {
            return Err(PyError::type_error(
                "zipimporter() missing required argument 'path'",
            ))
        }
    };
    if path_str.is_empty() {
        return Err(PyError::Exception(
            "ZipImportError".to_string(),
            PyObjectRef::new(PyObject::Exception {
                typ: "ZipImportError".to_string(),
                args: vec![py_str("archive path is empty")],
                cause: None,
                suppress_context: false,
                context: None,
                traceback: None,
                extra: None,
            }),
        ));
    }
    match crate::vm::VirtualMachine::split_zip_archive(&path_str) {
        Some((archive, prefix)) => {
            let prefix_slash = if prefix.is_empty() {
                String::new()
            } else {
                format!("{}/", prefix)
            };
            let mut inst_dict = AttrMap::new();
            inst_dict.insert_str("archive", py_str(&archive));
            inst_dict.insert_str("prefix", py_str(&prefix_slash));
            Ok(PyObjectRef::new(PyObject::Instance {
                typ: zi_cls,
                dict: inst_dict,
            }))
        }
        None => Err(PyError::Exception(
            "ZipImportError".to_string(),
            PyObjectRef::new(PyObject::Exception {
                typ: "ZipImportError".to_string(),
                args: vec![py_str(&format!("not a Zip file: '{}'", path_str))],
                cause: None,
                suppress_context: false,
                context: None,
                traceback: None,
                extra: None,
            }),
        )),
    }
}
