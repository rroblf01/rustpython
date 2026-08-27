use crate::object::*;
use std::collections::HashMap;
use std::sync::atomic::AtomicI64;
use num_traits::{Signed, ToPrimitive};
use std::rc::Rc;

use super::abc::create_abc_builtins_dict;
use super::codecs::create_codecs_dict;
use super::math::create_math_dict;


pub fn create_builtins() -> HashMap<String, PyObjectRef> {
    let mut builtins = HashMap::new();
    builtins.insert_str("None", py_none());
    builtins.insert_str("True", py_bool(true));
    builtins.insert_str("False", py_bool(false));
    // `__debug__` — always True here (no `-O` optimize-flag equivalent to
    // turn it off), used by real code as `if __debug__: assert ...`-style
    // guards and by the `assert` statement's own real-CPython semantics.
    builtins.insert_str("__debug__", py_bool(true));
    builtins.insert_str(
        "Ellipsis",
        PyObjectRef::imm(PyObject::Str(compact_str::CompactString::from("..."))),
    );
    // NotImplemented: the singleton rich-comparison/binary-op dunders return
    // to signal "try the other operand's reflected method instead" — needed
    // by any `__eq__`/`__lt__`/etc. that follows the standard pattern of
    // `if not isinstance(other, X): return NotImplemented`.
    {
        let mut nie_dict = HashMap::new();
        nie_dict.insert_str(
            "__repr__",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "__repr__".to_string(),
                func: |_args| Ok(py_str("NotImplemented")),
            }),
        );
        nie_dict.insert_str(
            "__bool__",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "__bool__".to_string(),
                func: |_args| Ok(py_bool(true)),
            }),
        );
        let nie_type = PyObjectRef::new(PyObject::Type {
            name: "NotImplementedType".to_string(),
            dict: Box::new(str_map_to_typedict(nie_dict)),
            bases: vec![],
            mro: vec![],
        });
        let not_implemented = PyObjectRef::imm(PyObject::Instance {
            typ: nie_type,
            dict: AttrMap::new(),
        });
        crate::object::seed_not_implemented(not_implemented.clone());
        builtins.insert_str("NotImplemented", not_implemented);
    }

    macro_rules! add_func {
        ($name:expr, $func:expr) => {
            builtins.insert(
                $name.to_string(),
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: $name.to_string(),
                    func: $func,
                }),
            );
        };
    }

    add_func!("print", builtin_print);
    add_func!("len", builtin_len);
    add_func!("range", builtin_range);
    // "type" is registered further down as a real, subclassable Type object
    // once `object_type` exists — see the comment there. "int"/"str"/"list"/
    // "float"/"dict"/"tuple"/"bytes"/"set"/"complex"/"bytearray"/"frozenset"/
    // "bool" are likewise registered further down (once `object_type`
    // exists) as real Types — see the comments there.
    add_func!("abs", builtin_abs);
    add_func!("hasattr", builtin_hasattr);
    add_func!("getattr", builtin_getattr);
    add_func!("setattr", builtin_setattr);
    add_func!("delattr", builtin_delattr);
    add_func!("ord", builtin_ord);
    add_func!("chr", builtin_chr);
    add_func!("hex", builtin_hex);
    add_func!("oct", builtin_oct);
    add_func!("bin", builtin_bin);
    add_func!("ascii", builtin_ascii);
    add_func!("memoryview", builtin_memoryview);
    add_func!("input", builtin_input);
    add_func!("exit", builtin_exit);
    add_func!("repr", builtin_repr);
    add_func!("sorted", builtin_sorted);
    add_func!("enumerate", builtin_enumerate);
    add_func!("iter", builtin_iter);
    add_func!("next", builtin_next);
    // `anext(aiterator, default=...)` (3.10+) — the async equivalent of
    // `next()`, was missing entirely (`NameError`). Real semantics: calls
    // `aiterator.__anext__()` and returns the resulting AWAITABLE (the
    // caller does `await anext(ait)`, matching `await ait.__anext__()`
    // exactly) — unlike `next()`, this function itself does no synchronous
    // driving/StopIteration-catching, since the iterator's `__anext__` is
    // itself async. The 2-arg default form additionally needs the returned
    // awaitable to substitute `default` if `StopAsyncIteration` occurs
    // WHEN AWAITED — not implemented here (would need a dedicated wrapper
    // awaitable type this codebase doesn't have); this covers the far more
    // common 1-arg form (`await anext(ait)`) and passes the default-arg
    // form's `__anext__()` awaitable straight through unwrapped, so a
    // `StopAsyncIteration` still propagates instead of substituting the
    // default — a known, deliberate partial gap in the same category as
    // this codebase's other async-generator internals limitations.
    add_func!("anext", |args: &[PyObjectRef]| {
        if args.is_empty() {
            return Err(PyError::type_error("anext() takes at least 1 argument"));
        }
        let f = args[0].borrow().get_attribute("__anext__").map_err(|_| {
            PyError::type_error(format!(
                "'{}' object is not an async iterator",
                args[0].borrow().type_name()
            ))
        })?;
        crate::object::call_bound_method(f, args[0].clone(), vec![])
    });
    add_func!("sum", builtin_sum);
    add_func!("max", builtin_max);
    add_func!("min", builtin_min);
    add_func!("id", builtin_id);
    add_func!("vars", builtin_vars);
    add_func!("isinstance", builtin_isinstance);
    add_func!("open", builtin_open);
    add_func!("any", builtin_any);
    add_func!("all", builtin_all);
    add_func!("callable", builtin_callable);
    add_func!("breakpoint", builtin_breakpoint);
    add_func!("pow", builtin_pow);
    add_func!("reversed", builtin_reversed);
    add_func!("issubclass", builtin_issubclass);
    add_func!("help", builtin_help);
    add_func!("eval", builtin_eval);
    add_func!("exec", builtin_exec);
    add_func!("__import__", builtin_import);
    add_func!("compile", builtin_compile);
    add_func!("super", builtin_super);
    // Internal helper the compiler emits IN PLACE OF a normal `super()` call
    // when a bare, zero-arg `super()` appears inside a function that itself
    // takes no parameters at all — there is then genuinely no `self` to
    // bind, matching real CPython's `RuntimeError: super(): no arguments`
    // (as opposed to a class-body method with parameters, which always gets
    // `__class__`/`self` injected at compile time and never reaches this).
    // Not a real builtin — never referenced by name from user code, only
    // ever emitted directly by `compile_expr`'s PEP 3135 handling.
    add_func!("__super_no_arguments_error", |_args: &[PyObjectRef]| {
        Err(PyError::runtime_error("super(): no arguments"))
    });
    add_func!("map", builtin_map);
    add_func!("filter", builtin_filter);
    add_func!("zip", builtin_zip);
    add_func!("property", builtin_property);
    add_func!("staticmethod", builtin_staticmethod);
    add_func!("classmethod", builtin_classmethod);
    add_func!("format", builtin_format);
    add_func!("object", builtin_object);
    add_func!("hash", builtin_hash);
    add_func!("slice", builtin_slice);
    add_func!("divmod", builtin_divmod);
    add_func!("round", builtin_round);
    add_func!("dir", builtin_dir);
    add_func!("globals", builtin_globals);
    add_func!("locals", builtin_locals);

    macro_rules! add_exc_type {
        ($name:expr, $func:expr) => {
            builtins.insert(
                $name.to_string(),
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: $name.to_string(),
                    func: $func,
                }),
            );
        };
    }

    add_exc_type!("BaseException", builtin_make_exception_baseexception);
    add_exc_type!("Exception", builtin_make_exception_exception);
    add_exc_type!("TypeError", builtin_make_exception_typeerror);
    add_exc_type!("ValueError", builtin_make_exception_valueerror);
    add_exc_type!(
        "ZeroDivisionError",
        builtin_make_exception_zerodivisionerror
    );
    add_exc_type!("NameError", builtin_make_exception_nameerror);
    add_exc_type!(
        "UnboundLocalError",
        builtin_make_exception_unboundlocalerror
    );
    add_exc_type!("AttributeError", builtin_make_exception_attributeerror);
    add_exc_type!("IndexError", builtin_make_exception_indexerror);
    add_exc_type!("KeyError", builtin_make_exception_keyerror);
    add_exc_type!("RuntimeError", builtin_make_exception_runtimeerror);
    add_exc_type!("StopIteration", builtin_make_exception_stopiteration);
    add_exc_type!("AssertionError", builtin_make_exception_assertionerror);
    add_exc_type!("OSError", builtin_make_exception_oserror);
    // `IOError`/`EnvironmentError` are ALIASES of `OSError` (real CPython:
    // `IOError is OSError` is True — they're bound to the SAME object and
    // their `__name__` is "OSError"). Registering them as separate classes
    // made `issubclass(IOError, BaseException)` find them as distinct
    // names, which test_baseexception's hierarchy audit flags as "not
    // accounted for" (the hierarchy file only lists OSError).
    {
        let oserror = builtins.get("OSError").cloned().unwrap_or_else(|| {
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "OSError".to_string(),
                func: builtin_make_exception_oserror,
            })
        });
        builtins.insert_str("EnvironmentError", oserror.clone());
        builtins.insert_str("IOError", oserror);
    }
    add_exc_type!("ImportError", builtin_make_exception_importerror);
    add_exc_type!("LookupError", builtin_make_exception_lookuperror);
    add_exc_type!("ArithmeticError", builtin_make_exception_arithmeticerror);
    add_exc_type!(
        "FloatingPointError",
        builtin_make_exception_floatingpointerror
    );
    add_exc_type!("OverflowError", builtin_make_exception_overflowerror);
    // EnvironmentError/IOError are registered as OSError aliases above.
    add_exc_type!(
        "FileNotFoundError",
        builtin_make_exception_filenotfounderror
    );
    add_exc_type!("PermissionError", builtin_make_exception_permissionerror);
    add_exc_type!(
        "NotImplementedError",
        builtin_make_exception_notimplementederror
    );
    add_exc_type!("RecursionError", builtin_make_exception_recursionerror);
    add_exc_type!(
        "PythonFinalizationError",
        builtin_make_exception_pythonfinalizationerror
    );
    add_exc_type!(
        "KeyboardInterrupt",
        builtin_make_exception_keyboardinterrupt
    );
    add_exc_type!("GeneratorExit", builtin_make_exception_generatorexit);
    add_exc_type!("SystemExit", builtin_make_exception_systemexit);
    add_exc_type!(
        "ModuleNotFoundError",
        builtin_make_exception_modulenotfounderror
    );
    add_exc_type!(
        "StopAsyncIteration",
        builtin_make_exception_stopasynciteration
    );
    add_exc_type!("EOFError", builtin_make_exception_eoferror);
    add_exc_type!("SyntaxError", builtin_make_exception_syntaxerror);
    add_exc_type!(
        "_IncompleteInputError",
        builtin_make_exception_incompleteinputerror
    );
    add_exc_type!("ConnectionError", builtin_make_exception_connectionerror);
    add_exc_type!("BrokenPipeError", builtin_make_exception_brokenpipeerror);
    add_exc_type!(
        "ConnectionRefusedError",
        builtin_make_exception_connectionrefusederror
    );
    add_exc_type!("BlockingIOError", builtin_make_exception_blockingioerror);
    add_exc_type!(
        "ChildProcessError",
        builtin_make_exception_childprocesserror
    );
    add_exc_type!("InterruptedError", builtin_make_exception_interruptederror);
    add_exc_type!("TimeoutError", builtin_make_exception_timeouterror);
    add_exc_type!("UnicodeError", builtin_make_exception_unicodeerror);
    add_exc_type!(
        "UnicodeDecodeError",
        builtin_make_exception_unicodedecodeerror
    );
    add_exc_type!(
        "UnicodeEncodeError",
        builtin_make_exception_unicodeencodeerror
    );
    add_exc_type!("ExceptionGroup", builtin_make_exception_exceptiongroup);
    add_exc_type!(
        "BaseExceptionGroup",
        builtin_make_exception_baseexceptiongroup
    );
    add_exc_type!("SystemError", builtin_make_exception_systemerror);
    add_exc_type!("Warning", builtin_make_exception_warning);
    add_exc_type!("UserWarning", builtin_make_exception_userwarning);
    add_exc_type!(
        "DeprecationWarning",
        builtin_make_exception_deprecationwarning
    );
    add_exc_type!(
        "PendingDeprecationWarning",
        builtin_make_exception_pendingdeprecationwarning
    );
    add_exc_type!("SyntaxWarning", builtin_make_exception_syntaxwarning);
    add_exc_type!("RuntimeWarning", builtin_make_exception_runtimewarning);
    add_exc_type!("FutureWarning", builtin_make_exception_futurewarning);
    add_exc_type!("ImportWarning", builtin_make_exception_importwarning);
    add_exc_type!("UnicodeWarning", builtin_make_exception_unicodewarning);
    add_exc_type!("EncodingWarning", builtin_make_exception_encodingwarning);
    add_exc_type!("BytesWarning", builtin_make_exception_byteswarning);
    add_exc_type!("ResourceWarning", builtin_make_exception_resourcewarning);
    add_exc_type!("ReferenceError", builtin_make_exception_referenceerror);
    add_exc_type!("BufferError", builtin_make_exception_buffererror);
    add_exc_type!("MemoryError", builtin_make_exception_memoryerror);
    add_exc_type!(
        "NotADirectoryError",
        builtin_make_exception_notadirectoryerror
    );
    add_exc_type!(
        "IsADirectoryError",
        builtin_make_exception_isadirectoryerror
    );
    add_exc_type!("FileExistsError", builtin_make_exception_fileexistserror);
    add_exc_type!(
        "ConnectionAbortedError",
        builtin_make_exception_connectionabortederror
    );
    add_exc_type!(
        "ConnectionResetError",
        builtin_make_exception_connectionreseterror
    );
    add_exc_type!(
        "ProcessLookupError",
        builtin_make_exception_processlookuperror
    );
    add_exc_type!(
        "UnicodeTranslateError",
        builtin_make_exception_unicodetranslateerror
    );
    add_exc_type!("IndentationError", builtin_make_exception_indentationerror);
    add_exc_type!("TabError", builtin_make_exception_taberror);

    let math_module = PyObjectRef::new(PyObject::Module {
        name: "math".to_string(),
        dict: Box::new(str_map_to_typedict(create_math_dict())),
    });
    builtins.insert_str("math", math_module.clone());

    // ── _codecs (needed by encodings module) ────────────────────────────────
    let codecs_module = PyObjectRef::new(PyObject::Module {
        name: "_codecs".to_string(),
        dict: Box::new(str_map_to_typedict(create_codecs_dict())),
    });
    builtins.insert_str("_codecs", codecs_module.clone());

    // ── _abc (needed by abc.py for ABCMeta, used by io/__init__.py) ────────
    let abc_module = PyObjectRef::new(PyObject::Module {
        name: "_abc".to_string(),
        dict: Box::new(str_map_to_typedict(create_abc_builtins_dict())),
    });
    builtins.insert_str("_abc", abc_module.clone());

    // Create a proper object TYPE with basic dunder methods.
    // This is used as the implicit base class for all classes without explicit bases.
    let mut object_dict = HashMap::new();
    // __setattr__(self, name, value): sets an attribute on the instance
    object_dict.insert_str(
        "__setattr__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__setattr__".to_string(),
            func: |args| {
                if args.len() < 3 {
                    return Err(PyError::type_error(
                        "__setattr__ requires at least 3 arguments (self, name, value)",
                    ));
                }
                let name = args[1].str();
                args[0].borrow_mut().set_attribute(&name, args[2].clone())?;
                Ok(py_none())
            },
        }),
    );
    // __getattribute__(self, name): gets an attribute from the instance
    object_dict.insert_str(
        "__getattribute__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__getattribute__".to_string(),
            func: |args| {
                if args.len() < 2 {
                    return Err(PyError::type_error(
                        "__getattribute__ requires at least 2 arguments (self, name)",
                    ));
                }
                let name = args[1].str();
                // Use get_attribute_impl directly (not the ObjectAccess
                // wrapper, which already rewrote the error into a full
                // exception with a RECONSTRUCTED obj clone) so the real
                // `args[0]` can be attached with correct identity.
                use crate::object::ObjectAccess;
                match args[0].borrow().get_attribute_impl(&name) {
                    Err(PyError::AttributeError(msg)) => {
                        let mut extra = std::collections::HashMap::new();
                        extra.insert("name".to_string(), py_str(&name));
                        extra.insert("obj".to_string(), args[0].clone());
                        Err(PyError::Exception(
                            "AttributeError".to_string(),
                            PyObjectRef::new(PyObject::Exception {
                                typ: "AttributeError".to_string(),
                                args: vec![py_str(&msg)],
                                cause: None,
                                suppress_context: false,
                                context: None,
                                traceback: None,
                                extra: Some(extra),
                            }),
                        ))
                    }
                    other => other,
                }
            },
        }),
    );
    // __init__(self): no-op
    object_dict.insert_str(
        "__init__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__init__".to_string(),
            func: |_args| Ok(py_none()),
        }),
    );
    // __repr__(self): <object at 0x...>
    object_dict.insert_str(
        "__repr__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__repr__".to_string(),
            func: |args| {
                if args.is_empty() {
                    return Err(PyError::type_error(
                        "__repr__ requires at least 1 argument (self)",
                    ));
                }
                let obj = &args[0];
                let obj_ref = obj.borrow();
                // Real CPython: `<module.ClassName object at 0x...>` —
                // dataclasses' repr=False and test_pprint's regex expect the
                // module-qualified name (test_dataclass_no_repr).
                let type_name = match &*obj_ref {
                    PyObject::Instance { typ, .. } => {
                        let tb = typ.borrow();
                        if let PyObject::Type { dict, name, .. } = &*tb {
                            let module = dict
                                .get_str("__module__")
                                .map(|m| m.str())
                                .unwrap_or_else(|| "builtins".to_string());
                            format!("{}.{}", module, name)
                        } else {
                            tb.type_name().to_string()
                        }
                    }
                    _ => obj_ref.type_name().to_string(),
                };
                let ptr = format!("{:p}", &*obj_ref as *const _ as *const u8);
                // Only show hex digits after 0x
                let ptr_hex = &ptr[2..];
                Ok(py_str(&format!("<{} object at 0x{}>", type_name, ptr_hex)))
            },
        }),
    );
    // `object.__str__` — real CPython's default just delegates to
    // `__repr__` (a subclass overriding only `__repr__`, not `__str__`,
    // still gets a sensible `str()`). Was missing entirely — `object.
    // __str__` (accessed as a CLASS attribute, e.g. `__str__ =
    // object.__str__`, a real idiom for explicitly opting back into the
    // default identity-based string form — real trigger: CPython's own
    // `xmlrpc/client.py`, `class Error(Exception): __str__ =
    // object.__str__`) raised `AttributeError: 'object' object has no
    // attribute '__str__'` even though plain `str(some_instance)` already
    // worked fine via this interpreter's own internal generic fallback
    // (that fallback was never reified as a real, gettable attribute on
    // the `object` type itself).
    object_dict.insert_str(
        "__str__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__str__".to_string(),
            func: |args| {
                if args.is_empty() {
                    return Err(PyError::type_error(
                        "__str__ requires at least 1 argument (self)",
                    ));
                }
                Ok(py_str(&args[0].repr()))
            },
        }),
    );
    // __eq__(self, other): identity comparison, deferring to the other
    // side (via `NotImplemented`, NOT a hard `False`) when not identical —
    // matches real CPython's actual `object.__eq__` exactly. Returning a
    // definite `False` here (the previous behavior) meant this NEVER
    // deferred to a reflected `__eq__`/allowed `py_compare`'s rich-compare
    // dispatch (`ops_compare.rs`) to even consider the other operand,
    // since "not NotImplemented" always short-circuited as final. Also
    // fixed an even older bug: this used to compare type NAMES instead of
    // identity — i.e. any two *distinct* instances of the same plain class
    // (no custom __eq__ override) compared equal to each other, which
    // surfaced very visibly once enum members relied on it: `Color.RED ==
    // Color.GREEN` was `True` since both are just "instances of Color".
    object_dict.insert_str(
        "__eq__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__eq__".to_string(),
            func: |args| {
                if args.len() < 2 {
                    return Err(PyError::type_error("__eq__ requires 2 arguments"));
                }
                if args[0].is(&args[1]) {
                    Ok(py_bool(true))
                } else {
                    Ok(crate::object::py_not_implemented())
                }
            },
        }),
    );
    // __ne__(self, other): real CPython's default doesn't do its own
    // identity check — it delegates to `self.__eq__(other)` (whatever
    // `__eq__` is ACTUALLY bound to `self`'s real class, which may be a
    // subclass override) and inverts, propagating `NotImplemented`
    // unchanged if `__eq__` itself couldn't decide. The previous
    // hard-identity implementation bypassed any custom `__eq__` entirely
    // — real trigger: CPython's own `test_compare.py`'s
    // `test_ne_high_priority`/`test_ne_low_priority`, which rely on
    // `object.__ne__` consulting `self.__eq__` (and nothing else) to
    // determine `calls` ordering.
    object_dict.insert_str(
        "__ne__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__ne__".to_string(),
            func: |args| {
                if args.len() < 2 {
                    return Err(PyError::type_error("__ne__ requires 2 arguments"));
                }
                let self_obj = args[0].clone();
                let eq_method = if let PyObject::Instance { typ, .. } = &*self_obj.borrow() {
                    crate::object::lookup_dunder_via_mro(typ, "__eq__")
                } else {
                    None
                };
                match eq_method {
                    Some(f) => {
                        let result =
                            crate::object::call_bound_method(f, self_obj, vec![args[1].clone()])?;
                        if crate::object::is_not_implemented(&result) {
                            Ok(result)
                        } else {
                            Ok(py_bool(!result.truthy()))
                        }
                    }
                    // Shouldn't normally happen (every Instance's mro includes
                    // `object`, which always provides `__eq__`) — kept as a
                    // safe, identity-based fallback rather than panicking.
                    None => Ok(py_bool(!args[0].is(&args[1]))),
                }
            },
        }),
    );
    // __hash__(self): hash based on pointer (identity hash). Inline values
    // (SmallInt/SmallFloat/SmallStr/None) have no allocation to point at —
    // `&*args[0].borrow()` would hash a transient stack temporary, unstable
    // across calls — so those use their stable VALUE hash instead (which is
    // also what `hash()` reports for them, keeping `test_hash_nan`'s
    // `hash(nan) == object.__hash__(nan)` invariant intact).
    object_dict.insert_str(
        "__hash__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__hash__".to_string(),
            func: |args| {
                if args.is_empty() {
                    return Err(PyError::type_error(
                        "__hash__ requires at least 1 argument (self)",
                    ));
                }
                match &args[0] {
                    PyObjectRef::Mut(_) | PyObjectRef::Imm(_) => {
                        let ptr: *const PyObject = &*args[0].borrow();
                        Ok(py_int(ptr as i64))
                    }
                    _ => Ok(py_int(args[0].hash()? as i64)),
                }
            },
        }),
    );
    // __new__(cls, *extra): creates a new instance of cls. When cls
    // transparently subclasses a native type (list/dict/str/int — see
    // NATIVE_BASE_MARKER), the native backing must be populated here too,
    // using any extra positional args exactly like CPython's int.__new__
    // (cls, value)/str.__new__(cls, value) would — otherwise a bare
    // `object.__new__(cls)` on such a class produced an instance with NO
    // native backing at all (broken: every native-delegated operation on
    // it would fail). This is also the enum module's own construction
    // path for value-carrying members (`object.__new__(cls, value)` used
    // instead of `cls(value)`, since `cls(...)` is overridden by
    // EnumType.__call__ to mean value lookup, not construction).
    object_dict.insert_str(
        "__new__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__new__".to_string(),
            func: |args| {
                if args.is_empty() {
                    return Err(PyError::type_error(
                        "__new__ requires at least 1 argument (cls)",
                    ));
                }
                let cls = args[0].clone();
                // `int.__new__(bool, ...)` / `object.__new__(bool, ...)`
                // must TypeError — bool has its own allocator
                // (test_bool::test_subclass).
                if matches!(&*cls.borrow(), PyObject::Type { name, .. } if name == "bool") {
                    return Err(PyError::type_error(
                        "int.__new__(bool) is not safe, use bool.__new__()",
                    ));
                }
                let native_kind = if let PyObject::Type { dict, .. } = &*cls.borrow() {
                    dict.get_str(crate::object::NATIVE_BASE_MARKER)
                        .map(|v| v.str())
                } else {
                    None
                };
                let mut instance_dict = AttrMap::new();
                if let Some(kind) = &native_kind {
                    let native = crate::object::synthesize_native_init(kind, &args[1..], &[])?;
                    instance_dict.insert(crate::object::NATIVE_BACKING_KEY.to_string(), native);
                }
                Ok(PyObjectRef::new(PyObject::Instance {
                    typ: cls,
                    dict: instance_dict,
                }))
            },
        }),
    );
    // __init_subclass__(cls, **kwargs): no-op (PEP 487) but must reject unexpected kwargs
    object_dict.insert_str(
        "__init_subclass__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__init_subclass__".to_string(),
            func: |args| {
                if args.len() > 1 {
                    // Keywords are packed into trailing dict by call_function
                    if let Some(last) = args.last() {
                        if let PyObject::Dict(d) = &*last.borrow() {
                            if d.len() > 0 {
                                return Err(PyError::type_error(
                                    "object.__init_subclass__() takes no keyword arguments",
                                ));
                            }
                        }
                    }
                }
                // Also handle direct **kwargs dict as second arg
                if args.len() == 2 {
                    if let PyObject::Dict(d) = &*args[1].borrow() {
                        if d.len() > 0 {
                            return Err(PyError::type_error(
                                "object.__init_subclass__() takes no keyword arguments",
                            ));
                        }
                    }
                }
                Ok(py_none())
            },
        }),
    );
    // __class_getitem__(cls, item): for generic types like List[int] (PEP 560)
    object_dict.insert_str(
        "__class_getitem__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__class_getitem__".to_string(),
            func: |args| {
                if args.len() < 2 {
                    return Err(PyError::type_error(
                        "__class_getitem__ requires at least 2 arguments (cls, item)",
                    ));
                }
                // Build a real types.GenericAlias (previously returned a bare
                // (cls, item) tuple, so `dict[str, str] | None` — configparser.py's
                // class annotations — raised "unsupported operand types for |").
                let item = &args[1];
                let item_args = if let PyObject::Tuple(t) = &*item.borrow() {
                    t.clone()
                } else {
                    vec![item.clone()]
                };
                Ok(crate::modules::make_generic_alias(
                    args[0].clone(),
                    item_args,
                ))
            },
        }),
    );
    // __format__(self, format_spec): basic format support
    object_dict.insert_str(
        "__format__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__format__".to_string(),
            func: |args| {
                if args.is_empty() {
                    return Err(PyError::type_error(
                        "__format__ requires at least 1 argument (self)",
                    ));
                }
                let obj = &args[0];
                let spec = if args.len() > 1 {
                    args[1].str()
                } else {
                    String::new()
                };
                if spec.is_empty() {
                    Ok(py_str(&obj.repr()))
                } else {
                    Err(PyError::value_error(format!(
                        "unknown format code '{}' for object",
                        spec
                    )))
                }
            },
        }),
    );
    // __reduce__(self): basic pickle support
    object_dict.insert_str(
        "__reduce__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__reduce__".to_string(),
            func: |_args| Ok(py_none()),
        }),
    );
    let object_type = PyObjectRef::new(PyObject::Type {
        name: "object".to_string(),
        dict: Box::new(str_map_to_typedict(object_dict)),
        bases: vec![],
        mro: vec![],
    });
    // Set MRO so isinstance works
    if let PyObject::Type { mro, .. } = &mut *object_type.borrow_mut() {
        *mro = vec![object_type.clone()];
    }
    // Register in builtins both as a type (for __build_class__) and as a callable (for object())
    builtins.insert_str("object", object_type.clone());
    // Also keep the function for direct use
    builtins.insert_str(
        "_object_func",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "object".to_string(),
            func: builtin_object,
        }),
    );

    // `int` — the first of the "native value" types (int/str/list/dict/...)
    // migrated from a plain `PyObject::BuiltinFunction` constructor to a
    // real, subclassable `PyObject::Type`, closing the long-standing
    // "native types aren't real Type objects" gap for this one type (see
    // `NATIVE_VALUE_CTOR_KEY`'s doc comment in object.rs for the full
    // mechanism). `int_dict`'s `NATIVE_VALUE_CTOR_KEY` entry points at the
    // ORIGINAL native constructor closure (`builtin_int`) — `call_function`
    // dispatches through it and returns the raw, unwrapped `PyObject::Int`
    // result, so `int(5)` still produces a plain int, never an
    // instance-of-int wrapper. `from_bytes` is the one method that
    // genuinely needs to live in this dict now (every other int method
    // like `bit_length` keeps resolving via native-backing delegation on a
    // `class MyInt(int)` instance, unaffected by this change).
    let mut int_dict: HashMap<String, PyObjectRef> = HashMap::new();
    int_dict.insert_str(
        crate::object::NATIVE_VALUE_CTOR_KEY,
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "int".to_string(),
            func: builtin_int,
        }),
    );
    int_dict.insert_str(
        "from_bytes",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "from_bytes".to_string(),
            func: crate::object::builtin_int_from_bytes,
        }),
    );
    int_dict.insert_str(
        "__init__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__init__".to_string(),
            func: crate::object::native_base_init_builtin,
        }),
    );
    int_dict.insert_str(
        "__repr__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__repr__".to_string(),
            func: crate::object::builtin_int_repr as BuiltinFunc,
        }),
    );
    // int.__format__(self, format_spec): format integer with format spec mini-language
    int_dict.insert_str(
        "__format__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__format__".to_string(),
            func: |args| {
                if args.is_empty() {
                    return Err(PyError::type_error(
                        "__format__ requires at least 1 argument (self)",
                    ));
                }
                let spec = if args.len() > 1 {
                    args[1].str()
                } else {
                    String::new()
                };
                if spec.is_empty() {
                    Ok(py_str(&args[0].repr()))
                } else {
                    crate::vm::format_with_spec(&args[0], &spec).map(|s| py_str(&s))
                }
            },
        }),
    );
    // int.__itemsize__ — Python arbitrary-precision ints have no fixed size
    int_dict.insert_str(
        "__itemsize__",
        PyObjectRef::new(PyObject::Property(Box::new(PropertyData {
            getter: Some(PyObjectRef::new(PyObject::BuiltinFunction {
                name: "__itemsize__".to_string(),
                func: |_args| Ok(py_int(0)),
            })),
            setter: None,
            deleter: None,
            doc: None,
        }))),
    );
    let int_type = PyObjectRef::new(PyObject::Type {
        name: "int".to_string(),
        dict: Box::new(str_map_to_typedict(int_dict)),
        bases: vec![object_type.clone()],
        mro: vec![],
    });
    if let PyObject::Type { mro, .. } = &mut *int_type.borrow_mut() {
        *mro = vec![int_type.clone(), object_type.clone()];
    }
    builtins.insert_str("int", int_type.clone());
    crate::object::seed_primitive_type_cache("int", int_type.clone());

    // `str` — the second type migrated to the `NATIVE_VALUE_CTOR_KEY`
    // mechanism, same shape as `int` above. Unlike `int` (which needed
    // `from_bytes` as a genuine class-level method), `str` has no method
    // that's reached ONLY at the class level — every instance method
    // (`.upper()`, `.join()`, etc.) keeps resolving via native-backing
    // delegation on `class MyStr(str)` instances, unaffected by this change
    // — so `str_dict` needs just the ctor marker.
    let mut str_dict: HashMap<String, PyObjectRef> = HashMap::new();
    str_dict.insert_str(
        crate::object::NATIVE_VALUE_CTOR_KEY,
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "str".to_string(),
            func: builtin_str,
        }),
    );
    str_dict.insert_str(
        "__init__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__init__".to_string(),
            func: crate::object::native_base_init_builtin,
        }),
    );
    str_dict.insert_str(
        "maketrans",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "maketrans".to_string(),
            func: crate::object::str_maketrans_builtin,
        }),
    );
    // `str.casefold` — the instance-level method resolves via the Str
    // get_attribute arm, but unbound CLASS-level access (`str.casefold`,
    // real CPython's own `test_bisect.py` uses it as a key function) failed
    // with AttributeError because the type dict only had the ctor marker.
    str_dict.insert_str(
        "casefold",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "casefold".to_string(),
            func: |args: &[PyObjectRef]| {
                if args.is_empty() {
                    return Err(PyError::type_error("casefold() missing required argument"));
                }
                Ok(py_str(&args[0].str().to_lowercase()))
            },
        }),
    );
    // Unbound CLASS-level access to the common str methods (`str.strip`,
    // `map(str.strip, ...)` — the standard idiom test_format_testfile uses).
    // Each delegates to the same instance method (resolved via get_attribute
    // on the first arg). The macro's per-name literal makes each closure
    // non-capturing (coerces to BuiltinFunc).
    macro_rules! str_unbound {
        ($name:literal) => {
            str_dict.insert_str(
                $name,
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: $name.to_string(),
                    func: |args: &[PyObjectRef]| {
                        if args.is_empty() {
                            return Err(PyError::type_error(concat!(
                                $name,
                                "() missing required argument: 'self'"
                            )));
                        }
                        // The instance method's BuiltinMethod carries a PLACEHOLDER
                        // self_obj (None) — the receiver is bound by the CALL
                        // machinery on `s.strip()`. Call the underlying func
                        // directly with the receiver as the first arg.
                        // Resolve the real method from the NATIVE BACKING when the
                        // receiver is a native-base subclass instance (`class
                        // Foo(str)`) — resolving through the instance's MRO
                        // returns THIS very same str_unbound closure, recursing
                        // forever (stack overflow on `Foo('hi').upper()`).
                        let method = match crate::object::native_backing_of(&args[0]) {
                            Some(native) => native.borrow().get_attribute($name)?,
                            None => args[0].borrow().get_attribute($name)?,
                        };
                        let is_builtin_method = {
                            let m = method.borrow();
                            matches!(&*m, PyObject::BuiltinMethod { .. })
                        };
                        if is_builtin_method {
                            let m = method.borrow();
                            if let PyObject::BuiltinMethod { func, .. } = &*m {
                                let mut all = vec![args[0].clone()];
                                all.extend_from_slice(&args[1..]);
                                return func(&all);
                            }
                        }
                        // Non-BuiltinMethod (e.g. an override from a subclass's
                        // dict) — call it with the receiver prepended.
                        let mut all = vec![args[0].clone()];
                        all.extend_from_slice(&args[1..]);
                        crate::object::call_function_disposable(&method, all, vec![])
                    },
                }),
            );
        };
    }
    str_unbound!("strip");
    str_unbound!("rstrip");
    str_unbound!("lstrip");
    str_unbound!("split");
    str_unbound!("rsplit");
    str_unbound!("upper");
    str_unbound!("lower");
    str_unbound!("replace");
    str_unbound!("format");
    str_unbound!("join");
    str_unbound!("startswith");
    str_unbound!("endswith");
    str_unbound!("find");
    str_unbound!("rfind");
    str_unbound!("index");
    str_unbound!("rindex");
    str_unbound!("count");
    str_unbound!("splitlines");
    str_unbound!("capitalize");
    str_unbound!("title");
    str_unbound!("swapcase");
    str_unbound!("zfill");
    str_unbound!("encode");
    str_unbound!("decode");
    str_dict.insert_str(
        "__repr__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__repr__".to_string(),
            func: crate::object::builtin_str_repr as BuiltinFunc,
        }),
    );
    let str_type = PyObjectRef::new(PyObject::Type {
        name: "str".to_string(),
        dict: Box::new(str_map_to_typedict(str_dict)),
        bases: vec![object_type.clone()],
        mro: vec![],
    });
    if let PyObject::Type { mro, .. } = &mut *str_type.borrow_mut() {
        *mro = vec![str_type.clone(), object_type.clone()];
    }
    builtins.insert_str("str", str_type.clone());
    crate::object::seed_primitive_type_cache("str", str_type);

    // `list` — same shape as `int`/`str` above. No method is reached ONLY
    // at the class level (unlike `int.from_bytes`) — every instance method
    // (`.append()`, `.sort()`, etc.) keeps resolving via native-backing
    // delegation on `class MyList(list)` instances — so `list_dict` needs
    // just the ctor marker.
    let mut list_dict: HashMap<String, PyObjectRef> = HashMap::new();
    list_dict.insert_str(
        crate::object::NATIVE_VALUE_CTOR_KEY,
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "list".to_string(),
            func: builtin_list,
        }),
    );
    list_dict.insert_str(
        "__init__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__init__".to_string(),
            func: crate::object::native_base_init_builtin,
        }),
    );
    list_dict.insert_str(
        "__repr__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__repr__".to_string(),
            func: crate::object::builtin_list_repr as BuiltinFunc,
        }),
    );
    let list_type = PyObjectRef::new(PyObject::Type {
        name: "list".to_string(),
        dict: Box::new(str_map_to_typedict(list_dict)),
        bases: vec![object_type.clone()],
        mro: vec![],
    });
    if let PyObject::Type { mro, .. } = &mut *list_type.borrow_mut() {
        *mro = vec![list_type.clone(), object_type.clone()];
    }
    builtins.insert_str("list", list_type.clone());
    crate::object::seed_primitive_type_cache("list", list_type);

    // `float` — same shape as `int`/`str`/`list` above, but unlike those
    // three, `float` DOES have genuine class-level-only methods (previously
    // reached via a `bf_name == "float" && name == "..."` special case in
    // `get_attribute_impl`, `attrs.rs` — that dispatch only ever fired when
    // the object being accessed WAS the bare `BuiltinFunction` itself, i.e.
    // `float.fromhex`/`float.hex`/`float.__getformat__`/`float.from_number`
    // called unbound; a plain float VALUE's own `.hex()`/`.is_integer()`
    // etc. go through a wholly separate `PyObject::Float(_)` instance arm,
    // unaffected by this migration) — those become unreachable once `float`
    // is a real `Type` (the generic `BuiltinFunction`-name dispatch never
    // fires for it again), so they must move into `float_dict` here instead.
    let mut float_dict: HashMap<String, PyObjectRef> = HashMap::new();
    float_dict.insert_str(
        crate::object::NATIVE_VALUE_CTOR_KEY,
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "float".to_string(),
            func: builtin_float,
        }),
    );
    float_dict.insert_str(
        "__getformat__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__getformat__".to_string(),
            func: |args: &[PyObjectRef]| {
                if args.is_empty() {
                    return Err(PyError::type_error(
                        "__getformat__() takes exactly one argument",
                    ));
                }
                if !matches!(&*args[0].borrow(), PyObject::Str(_)) {
                    return Err(PyError::type_error("__getformat__() argument must be str"));
                }
                match args[0].str().as_str() {
                    "double" | "float" => Ok(py_str("IEEE, little-endian")),
                    other => Err(PyError::value_error(format!(
                        "__getformat__() argument must be 'double' or 'float', not '{}'",
                        other
                    ))),
                }
            },
        }),
    );
    float_dict.insert_str(
        "fromhex",
        PyObjectRef::new(PyObject::ClassMethod {
            func: PyObjectRef::imm(PyObject::BuiltinFunction {
                name: "fromhex".to_string(),
                func: crate::object::float_fromhex,
            }),
        }),
    );
    float_dict.insert_str(
        "hex",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "hex".to_string(),
            func: crate::object::float_class_hex,
        }),
    );
    float_dict.insert_str(
        "from_number",
        PyObjectRef::new(PyObject::ClassMethod {
            func: PyObjectRef::imm(PyObject::BuiltinFunction {
                name: "from_number".to_string(),
                func: |args| {
                    // Bound as a classmethod: args[0] is the calling type, args[1]
                    // the number.
                    if args.len() < 2 {
                        return Err(PyError::type_error(
                            "float.from_number() takes exactly 1 argument",
                        ));
                    }
                    let cls = &args[0];
                    let number = &args[1];
                    let extract = |number: &PyObjectRef| -> PyResult<Option<f64>> {
                        let b = number.borrow();
                        match &*b {
                            PyObject::Float(f) => Ok(Some(*f)),
                            PyObject::Int(i) => Ok(Some(crate::object::bigint_to_float(i)?)),
                            PyObject::Bool(b2) => Ok(Some(if *b2 { 1.0 } else { 0.0 })),
                            PyObject::Complex(..)
                            | PyObject::Str(_)
                            | PyObject::Bytes(_)
                            | PyObject::ByteArray(_) => Ok(None),
                            PyObject::Instance { .. } => {
                                drop(b);
                                // A transparent float-subclass instance's native
                                // backing is its value.
                                if let Some(native) = crate::object::native_backing_of(number) {
                                    if let Some(f) = native.as_f64() {
                                        return Ok(Some(f));
                                    }
                                }
                                let typ = match &*number.borrow() {
                                    PyObject::Instance { typ, .. } => typ.clone(),
                                    _ => unreachable!(),
                                };
                                // __float__, then __index__ (CPython's PyFloat_AsDouble).
                                if let Some(f) =
                                    crate::object::lookup_dunder_via_mro(&typ, "__float__")
                                {
                                    let result = crate::object::call_bound_method(
                                        f,
                                        number.clone(),
                                        vec![],
                                    )?;
                                    return Ok(Some(result.as_f64().ok_or_else(|| {
                                        PyError::type_error("__float__ returned non-float")
                                    })?));
                                }
                                if let Some(f) =
                                    crate::object::lookup_dunder_via_mro(&typ, "__index__")
                                {
                                    let result = crate::object::call_bound_method(
                                        f,
                                        number.clone(),
                                        vec![],
                                    )?;
                                    let v = result.borrow();
                                    if let PyObject::Int(i) = &*v {
                                        return Ok(Some(i.to_f64().unwrap_or(0.0)));
                                    }
                                    return Err(PyError::type_error("__index__ returned non-int"));
                                }
                                Ok(None)
                            }
                            _ => Ok(None),
                        }
                    };
                    let value = extract(number)?.ok_or_else(|| {
                        PyError::type_error(format!(
                            "float.from_number() argument must be a number, not '{}'",
                            number.borrow().type_name()
                        ))
                    })?;
                    let is_plain_float =
                        matches!(&*cls.borrow(), PyObject::Type { name, .. } if name == "float");
                    if is_plain_float {
                        // Exact-float input returns the SAME object (CPython
                        // identity contract: float.from_number(NAN) is NAN).
                        if matches!(&*number.borrow(), PyObject::Float(_)) {
                            return Ok(number.clone());
                        }
                        return Ok(py_float(value));
                    }
                    // Subclass call: build a float-subclass instance carrying the
                    // value as its native backing (mirrors what `FloatSubclass(3.14)`
                    // produces; done directly here because re-entering the VM via
                    // `with_vm_mut` from inside this already-in-call chain is UB).
                    let mut dict = crate::object::AttrMap::new();
                    dict.insert(
                        crate::object::NATIVE_BACKING_KEY.to_string(),
                        py_float(value),
                    );
                    Ok(PyObjectRef::new(PyObject::Instance {
                        typ: cls.clone(),
                        dict,
                    }))
                },
            }),
        }),
    );
    float_dict.insert_str(
        "__init__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__init__".to_string(),
            func: crate::object::native_base_init_builtin,
        }),
    );
    // __hash__: float values hash via CPython's mod-2**61-1 double hash.
    // Present in the type dict so a transparent float SUBCLASS routes here
    // (MRO finds float.__hash__ before any later base's Python-level
    // __hash__), and so a subclass nan hashes by object identity — matching
    // `object.__hash__`'s pointer hash, as CPython 3.13+ requires.
    float_dict.insert_str(
        "__hash__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__hash__".to_string(),
            func: |args| {
                if args.is_empty() {
                    return Err(PyError::type_error(
                        "float.__hash__() takes exactly 1 argument",
                    ));
                }
                // The float VALUE is either the arg itself (SmallFloat/boxed
                // Float) or the native backing of a transparent subclass
                // instance.
                let v = {
                    let b = args[0].borrow();
                    match &*b {
                        PyObject::Float(f) => *f,
                        PyObject::Instance { .. } => {
                            let backing = crate::object::native_backing_of(&args[0]);
                            match backing {
                                Some(bk) => match &*bk.borrow() {
                                    PyObject::Float(f) => *f,
                                    _ => {
                                        return Err(PyError::type_error(
                                            "float.__hash__() argument has no float backing",
                                        ))
                                    }
                                },
                                None => {
                                    return Err(PyError::type_error(
                                        "float.__hash__() argument has no float backing",
                                    ))
                                }
                            }
                        }
                        _ => {
                            return Err(PyError::type_error(
                                "float.__hash__() argument must be float",
                            ))
                        }
                    }
                };
                if v.is_nan() {
                    // CPython hashes NaN by object identity.
                    let ptr: *const PyObject = &*args[0].borrow();
                    return Ok(py_int(ptr as i64));
                }
                Ok(py_int(crate::object::hash_double(v) as i64))
            },
        }),
    );
    float_dict.insert_str(
        "__repr__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__repr__".to_string(),
            func: crate::object::builtin_float_repr as BuiltinFunc,
        }),
    );
    let float_type = PyObjectRef::new(PyObject::Type {
        name: "float".to_string(),
        dict: Box::new(str_map_to_typedict(float_dict)),
        bases: vec![object_type.clone()],
        mro: vec![],
    });
    if let PyObject::Type { mro, .. } = &mut *float_type.borrow_mut() {
        *mro = vec![float_type.clone(), object_type.clone()];
    }
    builtins.insert_str("float", float_type.clone());
    crate::object::seed_primitive_type_cache("float", float_type);

    // `dict` — same shape as the others above. Unlike `list`/`str`
    // (nothing class-level-only), `dict` DOES have genuine class-level
    // methods previously reached via `bf_name == "dict" && name == "..."`
    // in `get_attribute_impl` (`attrs.rs`) — those become unreachable once
    // `dict` is a real `Type`, so `fromkeys`/`__setitem__`/`__getitem__`
    // move into `dict_dict` here instead.
    let mut dict_dict: HashMap<String, PyObjectRef> = HashMap::new();
    dict_dict.insert_str(
        crate::object::NATIVE_VALUE_CTOR_KEY,
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "dict".to_string(),
            func: builtin_dict,
        }),
    );
    dict_dict.insert_str(
        "fromkeys",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "fromkeys".to_string(),
            func: |args| {
                if args.is_empty() {
                    return Err(PyError::type_error("fromkeys() takes at least 1 argument"));
                }
                let keys = crate::object::collect_iterable(&args[0])?;
                let value = args.get(1).cloned().unwrap_or_else(py_none);
                let mut d = PyDict::new();
                for k in keys {
                    d.set(k, value.clone())?;
                }
                Ok(PyObjectRef::new(PyObject::Dict(Box::new(d))))
            },
        }),
    );
    dict_dict.insert_str(
        "__setitem__",
        PyObjectRef::imm(PyObject::BuiltinMethod {
            name: "__setitem__".to_string(),
            func: crate::object::builtin_dict_setitem as BuiltinFunc,
            self_obj: py_none(),
        }),
    );
    dict_dict.insert_str(
        "__getitem__",
        PyObjectRef::imm(PyObject::BuiltinMethod {
            name: "__getitem__".to_string(),
            func: crate::object::builtin_dict_getitem as BuiltinFunc,
            self_obj: py_none(),
        }),
    );
    dict_dict.insert_str(
        "__init__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__init__".to_string(),
            func: crate::object::native_base_init_builtin,
        }),
    );
    dict_dict.insert_str(
        "__repr__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__repr__".to_string(),
            func: crate::object::builtin_dict_repr as BuiltinFunc,
        }),
    );
    let dict_type = PyObjectRef::new(PyObject::Type {
        name: "dict".to_string(),
        dict: Box::new(str_map_to_typedict(dict_dict)),
        bases: vec![object_type.clone()],
        mro: vec![],
    });
    if let PyObject::Type { mro, .. } = &mut *dict_type.borrow_mut() {
        *mro = vec![dict_type.clone(), object_type.clone()];
    }
    builtins.insert_str("dict", dict_type.clone());
    crate::object::seed_primitive_type_cache("dict", dict_type);

    // `tuple` — same shape as `list`/`str` (no class-level-only method).
    // Unlike those two, `tuple` was NOT previously in
    // `is_recognized_native_base_name`/`make_native_backing`/
    // `synthesize_native_init` at all — `class MyTuple(tuple): ...` was
    // silently broken before this migration added it there too (same class
    // of gap found and fixed for `float`; see the memory entry for this
    // migration effort).
    let mut tuple_dict: HashMap<String, PyObjectRef> = HashMap::new();
    tuple_dict.insert_str(
        crate::object::NATIVE_VALUE_CTOR_KEY,
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "tuple".to_string(),
            func: builtin_tuple,
        }),
    );
    tuple_dict.insert_str(
        "__init__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__init__".to_string(),
            func: crate::object::native_base_init_builtin,
        }),
    );
    tuple_dict.insert_str(
        "__repr__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__repr__".to_string(),
            func: crate::object::builtin_tuple_repr as BuiltinFunc,
        }),
    );
    let tuple_type = PyObjectRef::new(PyObject::Type {
        name: "tuple".to_string(),
        dict: Box::new(str_map_to_typedict(tuple_dict)),
        bases: vec![object_type.clone()],
        mro: vec![],
    });
    if let PyObject::Type { mro, .. } = &mut *tuple_type.borrow_mut() {
        *mro = vec![tuple_type.clone(), object_type.clone()];
    }
    builtins.insert_str("tuple", tuple_type.clone());
    crate::object::seed_primitive_type_cache("tuple", tuple_type);

    // `bytes` — same shape as `list`/`str`/`tuple` (no dunder-shaped
    // class-level method — `fromhex` is a plain classmethod-style
    // function, not a dunder, so it carries none of the ancestor-mro
    // hazards documented for `dict`'s migration).
    let mut bytes_dict: HashMap<String, PyObjectRef> = HashMap::new();
    bytes_dict.insert_str(
        crate::object::NATIVE_VALUE_CTOR_KEY,
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "bytes".to_string(),
            func: builtin_bytes,
        }),
    );
    bytes_dict.insert_str(
        "fromhex",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "fromhex".to_string(),
            func: builtin_bytes_fromhex,
        }),
    );
    bytes_dict.insert_str(
        "maketrans",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "maketrans".to_string(),
            func: crate::object::bytes_maketrans_builtin,
        }),
    );
    bytes_dict.insert_str(
        "__init__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__init__".to_string(),
            func: crate::object::native_base_init_builtin,
        }),
    );
    bytes_dict.insert_str(
        "__repr__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__repr__".to_string(),
            func: crate::object::builtin_bytes_repr as BuiltinFunc,
        }),
    );
    let bytes_type = PyObjectRef::new(PyObject::Type {
        name: "bytes".to_string(),
        dict: Box::new(str_map_to_typedict(bytes_dict)),
        bases: vec![object_type.clone()],
        mro: vec![],
    });
    if let PyObject::Type { mro, .. } = &mut *bytes_type.borrow_mut() {
        *mro = vec![bytes_type.clone(), object_type.clone()];
    }
    builtins.insert_str("bytes", bytes_type.clone());
    crate::object::seed_primitive_type_cache("bytes", bytes_type);

    // `set` — same shape, no class-level-only method at all.
    let mut set_dict: HashMap<String, PyObjectRef> = HashMap::new();
    set_dict.insert_str(
        crate::object::NATIVE_VALUE_CTOR_KEY,
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "set".to_string(),
            func: builtin_set,
        }),
    );
    set_dict.insert_str(
        "__init__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__init__".to_string(),
            func: crate::object::native_base_init_builtin,
        }),
    );
    set_dict.insert_str(
        "__repr__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__repr__".to_string(),
            func: crate::object::builtin_set_repr as BuiltinFunc,
        }),
    );
    let set_type = PyObjectRef::new(PyObject::Type {
        name: "set".to_string(),
        dict: Box::new(str_map_to_typedict(set_dict)),
        bases: vec![object_type.clone()],
        mro: vec![],
    });
    if let PyObject::Type { mro, .. } = &mut *set_type.borrow_mut() {
        *mro = vec![set_type.clone(), object_type.clone()];
    }
    builtins.insert_str("set", set_type.clone());
    crate::object::seed_primitive_type_cache("set", set_type);

    // `complex` — same shape; `from_number` is a plain classmethod-style
    // function (not a dunder).
    let mut complex_dict: HashMap<String, PyObjectRef> = HashMap::new();
    complex_dict.insert_str(
        crate::object::NATIVE_VALUE_CTOR_KEY,
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "complex".to_string(),
            func: builtin_complex,
        }),
    );
    complex_dict.insert_str(
        "from_number",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "from_number".to_string(),
            func: |args| {
                if args.is_empty() {
                    return Err(PyError::type_error(
                        "complex.from_number() takes exactly 1 argument",
                    ));
                }
                let n = args[0].as_f64().unwrap_or(0.0);
                Ok(PyObjectRef::imm(PyObject::Complex(n, 0.0)))
            },
        }),
    );
    complex_dict.insert_str(
        "__init__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__init__".to_string(),
            func: crate::object::native_base_init_builtin,
        }),
    );
    complex_dict.insert_str(
        "__repr__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__repr__".to_string(),
            func: crate::object::builtin_complex_repr as BuiltinFunc,
        }),
    );
    let complex_type = PyObjectRef::new(PyObject::Type {
        name: "complex".to_string(),
        dict: Box::new(str_map_to_typedict(complex_dict)),
        bases: vec![object_type.clone()],
        mro: vec![],
    });
    if let PyObject::Type { mro, .. } = &mut *complex_type.borrow_mut() {
        *mro = vec![complex_type.clone(), object_type.clone()];
    }
    builtins.insert_str("complex", complex_type.clone());
    crate::object::seed_primitive_type_cache("complex", complex_type);

    // `bytearray` — same shape, no class-level-only method.
    let mut bytearray_dict: HashMap<String, PyObjectRef> = HashMap::new();
    bytearray_dict.insert_str(
        crate::object::NATIVE_VALUE_CTOR_KEY,
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "bytearray".to_string(),
            func: builtin_bytearray,
        }),
    );
    bytearray_dict.insert_str(
        "__init__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__init__".to_string(),
            func: crate::object::native_base_init_builtin,
        }),
    );
    bytearray_dict.insert_str(
        "__repr__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__repr__".to_string(),
            func: crate::object::builtin_bytearray_repr as BuiltinFunc,
        }),
    );
    let bytearray_type = PyObjectRef::new(PyObject::Type {
        name: "bytearray".to_string(),
        dict: Box::new(str_map_to_typedict(bytearray_dict)),
        bases: vec![object_type.clone()],
        mro: vec![],
    });
    if let PyObject::Type { mro, .. } = &mut *bytearray_type.borrow_mut() {
        *mro = vec![bytearray_type.clone(), object_type.clone()];
    }
    builtins.insert_str("bytearray", bytearray_type.clone());
    crate::object::seed_primitive_type_cache("bytearray", bytearray_type);

    // `frozenset` — same shape, no class-level-only method.
    let mut frozenset_dict: HashMap<String, PyObjectRef> = HashMap::new();
    frozenset_dict.insert_str(
        crate::object::NATIVE_VALUE_CTOR_KEY,
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "frozenset".to_string(),
            func: builtin_frozenset,
        }),
    );
    frozenset_dict.insert_str(
        "__init__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__init__".to_string(),
            func: crate::object::native_base_init_builtin,
        }),
    );
    frozenset_dict.insert_str(
        "__repr__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__repr__".to_string(),
            func: crate::object::builtin_frozenset_repr as BuiltinFunc,
        }),
    );
    let frozenset_type = PyObjectRef::new(PyObject::Type {
        name: "frozenset".to_string(),
        dict: Box::new(str_map_to_typedict(frozenset_dict)),
        bases: vec![object_type.clone()],
        mro: vec![],
    });
    if let PyObject::Type { mro, .. } = &mut *frozenset_type.borrow_mut() {
        *mro = vec![frozenset_type.clone(), object_type.clone()];
    }
    builtins.insert_str("frozenset", frozenset_type.clone());
    crate::object::seed_primitive_type_cache("frozenset", frozenset_type);

    // `bool` — a real `Type` (fixing `type(True) is bool`) but
    // DELIBERATELY excluded from `is_recognized_native_base_name` and given
    // NO entry in `make_native_backing`/`synthesize_native_init`: real
    // Python disallows subclassing `bool` at all (`TypeError: type 'bool'
    // is not an acceptable base type`) — `default_build_class` (`vm.rs`)
    // checks for this explicitly, by identity against this exact binding,
    // before it would otherwise reach the generic `NATIVE_VALUE_CTOR_KEY`
    // detection arm and wrongly treat `bool` as a valid native base. Bases
    // are `[int_type, object_type]`, matching real CPython's actual
    // hierarchy (`bool.__bases__ == (int,)`) — `__new__` (constructing a
    // bool from a truthiness-tested value) moves in from what used to be a
    // `bf_name == "bool" && name == "__new__"` special case in
    // `get_attribute_impl` (`attrs.rs`).
    let mut bool_dict: HashMap<String, PyObjectRef> = HashMap::new();
    bool_dict.insert_str(
        crate::object::NATIVE_VALUE_CTOR_KEY,
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "bool".to_string(),
            func: builtin_bool,
        }),
    );
    bool_dict.insert_str(
        "__new__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__new__".to_string(),
            func: |args| {
                if args.is_empty() {
                    return Ok(py_bool(false));
                }
                if args.len() >= 2 {
                    return Ok(py_bool(args[1].truthy()));
                }
                Ok(py_bool(false))
            },
        }),
    );
    // bool.from_bytes returns a REAL bool (False/True), unlike the int 0/1
    // int.from_bytes produces (test_bool's test_from_bytes asserts
    // `is False`/`is True`). Must be in bool's own dict — via int's MRO it
    // would resolve to the same object as int.from_bytes.
    bool_dict.insert_str(
        "from_bytes",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "from_bytes".to_string(),
            func: |args: &[PyObjectRef]| {
                let v = crate::object::builtin_int_from_bytes(args)?;
                Ok(py_bool(v.truthy()))
            },
        }),
    );
    bool_dict.insert_str(
        "__repr__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__repr__".to_string(),
            func: crate::object::builtin_bool_repr as BuiltinFunc,
        }),
    );
    let bool_type = PyObjectRef::new(PyObject::Type {
        name: "bool".to_string(),
        dict: Box::new(str_map_to_typedict(bool_dict)),
        bases: vec![int_type.clone()],
        mro: vec![],
    });
    if let PyObject::Type { mro, .. } = &mut *bool_type.borrow_mut() {
        *mro = vec![bool_type.clone(), int_type.clone(), object_type.clone()];
    }
    builtins.insert_str("bool", bool_type.clone());
    crate::object::seed_primitive_type_cache("bool", bool_type);

    // `type` — a real, subclassable Type object (not just the `type(x)`
    // introspection/`type(name,bases,ns)` construction BuiltinFunction that
    // used to be the sole binding for this name), so `class MyMeta(type):
    // ...` works as a genuine metaclass with real MRO-based method
    // resolution (needed for e.g. enum's EnumType). `type.__new__` is what
    // a custom metaclass's `super().__new__(metacls, name, bases, ns,
    // **kwds)` bottoms out on. `type(x)` / `type(name, bases, ns)` calling
    // `type` itself still needs its own dual-arity behavior — that's kept
    // in `builtin_type_of` (object.rs) and special-cased at the top of
    // `call_function` (vm.rs) by identity against this exact object,
    // before the generic "call a Type to instantiate it" path would
    // otherwise try to build a plain Instance instead.
    let mut type_dict = HashMap::new();
    type_dict.insert_str(
        "__new__",
        PyObjectRef::new(PyObject::StaticMethod {
            func: PyObjectRef::new(PyObject::BuiltinFunction {
                name: "__new__".to_string(),
                func: crate::object::type_new_builtin,
            }),
        }),
    );
    let type_type = PyObjectRef::new(PyObject::Type {
        name: "type".to_string(),
        dict: Box::new(str_map_to_typedict(type_dict)),
        bases: vec![object_type.clone()],
        mro: vec![],
    });
    if let PyObject::Type { mro, .. } = &mut *type_type.borrow_mut() {
        *mro = vec![type_type.clone(), object_type];
    }
    builtins.insert_str("type", type_type);
    builtins.insert_str(
        "_type_func",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "type".to_string(),
            func: builtin_type_of,
        }),
    );

    builtins
}
