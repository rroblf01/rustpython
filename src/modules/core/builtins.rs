use crate::object::*;
use std::collections::HashMap;
use std::sync::atomic::AtomicI64;
use num_traits::{Signed, ToPrimitive};
use std::rc::Rc;

use super::abc::create_abc_builtins_dict;
use super::codecs::create_codecs_dict;
use super::math::create_math_dict;

mod primitive_types;
mod collection_types;


pub fn create_builtins() -> HashMap<String, PyObjectRef> {
    let mut builtins = HashMap::new();
    builtins.insert_str("None", py_none());
    builtins.insert_str("True", py_bool(true));
    builtins.insert_str("False", py_bool(false));
    // `__debug__` — always True here (no `-O` optimize-flag equivalent to
    // turn it off), used by real code as `if __debug__: assert ...`-style
    // guards and by the `assert` statement's own real-CPython semantics.
    builtins.insert_str("__debug__", py_bool(true));
    // Real CPython's `builtins` module has `__name__ == 'builtins'` in its
    // own `__dict__`, which is why `LOAD_NAME __name__`'s fallback to
    // `f.builtins` never fails at module scope there. Without this, a
    // frame whose globals lack `__name__` (e.g. a copy of a module's
    // `__dict__` used as exec/doctest globals) raises a spurious NameError
    // on the implicit `LOAD_NAME __name__` every `class` body prologue
    // emits.
    builtins.insert_str("__name__", py_str("builtins"));
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

    primitive_types::register_primitive_types(&mut builtins, &object_type);
    collection_types::register_collection_types(&mut builtins, &object_type);
    builtins
}
