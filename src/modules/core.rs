use crate::object::*;
use std::collections::HashMap;
use std::sync::atomic::AtomicI64;
use num_traits::Signed;

thread_local! {
    static CODEC_SEARCH_FUNCTIONS: std::cell::RefCell<Vec<crate::object::PyObjectRef>> = const { std::cell::RefCell::new(Vec::new()) };
}

// ── Safe wrappers for raw file descriptor operations ──────────────────────
// These encapsulate the `from_raw_fd` unsafe dereference so callers don't
// need `unsafe` blocks.  The fd ownership pattern is: create File, use it,
// then `forget()` to return ownership to the caller (who still owns the fd).

/// Read from a raw file descriptor without taking ownership of the fd.
fn read_fd(fd: i32, buf: &mut Vec<u8>) -> std::io::Result<usize> {
    use std::os::unix::io::FromRawFd;
    use std::io::Read;
    // SAFETY: from_raw_fd takes ownership, but we use forget() to return it.
    let mut f = unsafe { std::fs::File::from_raw_fd(fd) };
    let result = f.read(buf);
    std::mem::forget(f); // Don't close the fd — caller still owns it
    result
}

/// Write to a raw file descriptor without taking ownership of the fd.
fn write_fd(fd: i32, data: &[u8]) -> std::io::Result<usize> {
    use std::os::unix::io::FromRawFd;
    use std::io::Write;
    // SAFETY: from_raw_fd takes ownership, but we use forget() to return it.
    let mut f = unsafe { std::fs::File::from_raw_fd(fd) };
    let result = f.write(data);
    std::mem::forget(f);
    result
}

/// Close a raw file descriptor by wrapping it in a File and dropping it.
fn close_fd(fd: i32) {
    use std::os::unix::io::FromRawFd;
    // SAFETY: from_raw_fd takes ownership; dropping it below closes the fd.
    let file = unsafe { std::fs::File::from_raw_fd(fd) };
    drop(file); // Closes the fd
}

/// Get an independently-owned, safely-droppable `File` for a standard
/// stream fd (0/1/2) without ever opening `/dev/stdout`-style paths: doing
/// so via `File::create` implies `O_TRUNC`, and every `VirtualMachine::new()`
/// (including the disposable, throwaway VMs Rust builtins spin up to invoke
/// a Python-level method — see `call_bound_method`) rebuilds `sys.stdout`,
/// truncating the *real* process stdout out from under any output already
/// written by the outer, real VM. `try_clone()` duplicates the fd instead
/// (like `dup()`), sharing the real stream's file offset without truncating
/// it and without risking the real fd getting closed when this VM drops.
fn dup_std_fd(fd: i32) -> std::io::Result<std::fs::File> {
    use std::os::unix::io::FromRawFd;
    // SAFETY: from_raw_fd takes ownership, but we forget() it right after
    // cloning so the real fd (0/1/2) is never closed by this wrapper.
    let borrowed = unsafe { std::fs::File::from_raw_fd(fd) };
    let dup = borrowed.try_clone();
    std::mem::forget(borrowed);
    dup
}

pub fn create_builtins() -> HashMap<String, PyObjectRef> {
    let mut builtins = HashMap::new();
    builtins.insert_str("None", py_none());
    builtins.insert_str("True", py_bool(true));
    builtins.insert_str("False", py_bool(false));
    // `__debug__` — always True here (no `-O` optimize-flag equivalent to
    // turn it off), used by real code as `if __debug__: assert ...`-style
    // guards and by the `assert` statement's own real-CPython semantics.
    builtins.insert_str("__debug__", py_bool(true));
    builtins.insert_str("Ellipsis", PyObjectRef::imm(PyObject::Str(compact_str::CompactString::from("..."))));
    // NotImplemented: the singleton rich-comparison/binary-op dunders return
    // to signal "try the other operand's reflected method instead" — needed
    // by any `__eq__`/`__lt__`/etc. that follows the standard pattern of
    // `if not isinstance(other, X): return NotImplemented`.
    {
        let mut nie_dict = HashMap::new();
        nie_dict.insert_str("__repr__", PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__repr__".to_string(),
            func: |_args| Ok(py_str("NotImplemented")),
        }));
        nie_dict.insert_str("__bool__", PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__bool__".to_string(),
            func: |_args| Ok(py_bool(true)),
        }));
        let nie_type = PyObjectRef::new(PyObject::Type { name: "NotImplementedType".to_string(), dict: Box::new(str_map_to_typedict(nie_dict)), bases: vec![], mro: vec![] });
        let not_implemented = PyObjectRef::imm(PyObject::Instance { typ: nie_type, dict: AttrMap::new() });
        crate::object::seed_not_implemented(not_implemented.clone());
        builtins.insert_str("NotImplemented", not_implemented);
    }

    macro_rules! add_func {
        ($name:expr, $func:expr) => {
            builtins.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
                name: $name.to_string(),
                func: $func,
            }));
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
            builtins.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
                name: $name.to_string(),
                func: $func,
            }));
        };
    }

    add_exc_type!("BaseException", builtin_make_exception_baseexception);
    add_exc_type!("Exception", builtin_make_exception_exception);
    add_exc_type!("TypeError", builtin_make_exception_typeerror);
    add_exc_type!("ValueError", builtin_make_exception_valueerror);
    add_exc_type!("ZeroDivisionError", builtin_make_exception_zerodivisionerror);
    add_exc_type!("NameError", builtin_make_exception_nameerror);
    add_exc_type!("AttributeError", builtin_make_exception_attributeerror);
    add_exc_type!("IndexError", builtin_make_exception_indexerror);
    add_exc_type!("KeyError", builtin_make_exception_keyerror);
    add_exc_type!("RuntimeError", builtin_make_exception_runtimeerror);
    add_exc_type!("StopIteration", builtin_make_exception_stopiteration);
    add_exc_type!("AssertionError", builtin_make_exception_assertionerror);
    add_exc_type!("OSError", builtin_make_exception_oserror);
    add_exc_type!("ImportError", builtin_make_exception_importerror);
    add_exc_type!("LookupError", builtin_make_exception_lookuperror);
    add_exc_type!("ArithmeticError", builtin_make_exception_arithmeticerror);
    add_exc_type!("FloatingPointError", builtin_make_exception_floatingpointerror);
    add_exc_type!("OverflowError", builtin_make_exception_overflowerror);
    add_exc_type!("EnvironmentError", builtin_make_exception_environmenterror);
    add_exc_type!("IOError", builtin_make_exception_ioerror);
    add_exc_type!("FileNotFoundError", builtin_make_exception_filenotfounderror);
    add_exc_type!("PermissionError", builtin_make_exception_permissionerror);
    add_exc_type!("NotImplementedError", builtin_make_exception_notimplementederror);
    add_exc_type!("RecursionError", builtin_make_exception_recursionerror);
    add_exc_type!("KeyboardInterrupt", builtin_make_exception_keyboardinterrupt);
    add_exc_type!("GeneratorExit", builtin_make_exception_generatorexit);
    add_exc_type!("SystemExit", builtin_make_exception_systemexit);
    add_exc_type!("ModuleNotFoundError", builtin_make_exception_modulenotfounderror);
    add_exc_type!("StopAsyncIteration", builtin_make_exception_stopasynciteration);
    add_exc_type!("EOFError", builtin_make_exception_eoferror);
    add_exc_type!("SyntaxError", builtin_make_exception_syntaxerror);
    add_exc_type!("ConnectionError", builtin_make_exception_connectionerror);
    add_exc_type!("BrokenPipeError", builtin_make_exception_brokenpipeerror);
    add_exc_type!("ConnectionRefusedError", builtin_make_exception_connectionrefusederror);
    add_exc_type!("BlockingIOError", builtin_make_exception_blockingioerror);
    add_exc_type!("ChildProcessError", builtin_make_exception_childprocesserror);
    add_exc_type!("InterruptedError", builtin_make_exception_interruptederror);
    add_exc_type!("TimeoutError", builtin_make_exception_timeouterror);
    add_exc_type!("UnicodeError", builtin_make_exception_unicodeerror);
    add_exc_type!("UnicodeDecodeError", builtin_make_exception_unicodedecodeerror);
    add_exc_type!("UnicodeEncodeError", builtin_make_exception_unicodeencodeerror);
    add_exc_type!("ExceptionGroup", builtin_make_exception_exceptiongroup);
    add_exc_type!("BaseExceptionGroup", builtin_make_exception_baseexceptiongroup);
    add_exc_type!("SystemError", builtin_make_exception_systemerror);
    add_exc_type!("Warning", builtin_make_exception_warning);
    add_exc_type!("UserWarning", builtin_make_exception_userwarning);
    add_exc_type!("DeprecationWarning", builtin_make_exception_deprecationwarning);
    add_exc_type!("PendingDeprecationWarning", builtin_make_exception_pendingdeprecationwarning);
    add_exc_type!("SyntaxWarning", builtin_make_exception_syntaxwarning);
    add_exc_type!("RuntimeWarning", builtin_make_exception_runtimewarning);
    add_exc_type!("FutureWarning", builtin_make_exception_futurewarning);
    add_exc_type!("ImportWarning", builtin_make_exception_importwarning);
    add_exc_type!("UnicodeWarning", builtin_make_exception_unicodewarning);
    add_exc_type!("BytesWarning", builtin_make_exception_byteswarning);
    add_exc_type!("ResourceWarning", builtin_make_exception_resourcewarning);
    add_exc_type!("ReferenceError", builtin_make_exception_referenceerror);
    add_exc_type!("BufferError", builtin_make_exception_buffererror);
    add_exc_type!("MemoryError", builtin_make_exception_memoryerror);
    add_exc_type!("NotADirectoryError", builtin_make_exception_notadirectoryerror);
    add_exc_type!("IsADirectoryError", builtin_make_exception_isadirectoryerror);
    add_exc_type!("FileExistsError", builtin_make_exception_fileexistserror);
    add_exc_type!("ConnectionAbortedError", builtin_make_exception_connectionabortederror);
    add_exc_type!("ConnectionResetError", builtin_make_exception_connectionreseterror);
    add_exc_type!("ProcessLookupError", builtin_make_exception_processlookuperror);
    add_exc_type!("UnicodeTranslateError", builtin_make_exception_unicodetranslateerror);
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
    object_dict.insert_str("__setattr__", PyObjectRef::new(PyObject::BuiltinFunction {
        name: "__setattr__".to_string(),
        func: |args| {
            if args.len() < 3 {
                return Err(PyError::type_error("__setattr__ requires at least 3 arguments (self, name, value)"));
            }
            let name = args[1].str();
            args[0].borrow_mut().set_attribute(&name, args[2].clone())?;
            Ok(py_none())
        },
    }));
    // __getattribute__(self, name): gets an attribute from the instance
    object_dict.insert_str("__getattribute__", PyObjectRef::new(PyObject::BuiltinFunction {
        name: "__getattribute__".to_string(),
        func: |args| {
            if args.len() < 2 {
                return Err(PyError::type_error("__getattribute__ requires at least 2 arguments (self, name)"));
            }
            let name = args[1].str();
            args[0].borrow().get_attribute(&name)
        },
    }));
    // __init__(self): no-op
    object_dict.insert_str("__init__", PyObjectRef::new(PyObject::BuiltinFunction {
        name: "__init__".to_string(),
        func: |_args| Ok(py_none()),
    }));
    // __repr__(self): <object at 0x...>
    object_dict.insert_str("__repr__", PyObjectRef::new(PyObject::BuiltinFunction {
        name: "__repr__".to_string(),
        func: |args| {
            if args.is_empty() {
                return Err(PyError::type_error("__repr__ requires at least 1 argument (self)"));
            }
            let obj = &args[0];
            let obj_ref = obj.borrow();
            let type_name = obj_ref.type_name();
            let ptr = format!("{:p}", &*obj_ref as *const _ as *const u8);
            // Only show hex digits after 0x
            let ptr_hex = &ptr[2..];
            Ok(py_str(&format!("<{} object at 0x{}>", type_name, ptr_hex)))
        },
    }));
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
    object_dict.insert_str("__str__", PyObjectRef::new(PyObject::BuiltinFunction {
        name: "__str__".to_string(),
        func: |args| {
            if args.is_empty() {
                return Err(PyError::type_error("__str__ requires at least 1 argument (self)"));
            }
            Ok(py_str(&args[0].repr()))
        },
    }));
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
    object_dict.insert_str("__eq__", PyObjectRef::new(PyObject::BuiltinFunction {
        name: "__eq__".to_string(),
        func: |args| {
            if args.len() < 2 {
                return Err(PyError::type_error("__eq__ requires 2 arguments"));
            }
            if args[0].is(&args[1]) { Ok(py_bool(true)) } else { Ok(crate::object::py_not_implemented()) }
        },
    }));
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
    object_dict.insert_str("__ne__", PyObjectRef::new(PyObject::BuiltinFunction {
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
                    let result = crate::object::call_bound_method(f, self_obj, vec![args[1].clone()])?;
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
    }));
    // __hash__(self): hash based on pointer
    object_dict.insert_str("__hash__", PyObjectRef::new(PyObject::BuiltinFunction {
        name: "__hash__".to_string(),
        func: |args| {
            if args.is_empty() {
                return Err(PyError::type_error("__hash__ requires at least 1 argument (self)"));
            }
            let ptr: *const PyObject = &*args[0].borrow();
            Ok(py_int(ptr as i64))
        },
    }));
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
    object_dict.insert_str("__new__", PyObjectRef::new(PyObject::BuiltinFunction {
        name: "__new__".to_string(),
        func: |args| {
            if args.is_empty() {
                return Err(PyError::type_error("__new__ requires at least 1 argument (cls)"));
            }
            let cls = args[0].clone();
            let native_kind = if let PyObject::Type { dict, .. } = &*cls.borrow() {
                dict.get_str(crate::object::NATIVE_BASE_MARKER).map(|v| v.str())
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
    }));
    // __init_subclass__(cls, **kwargs): no-op (PEP 487)
    object_dict.insert_str("__init_subclass__", PyObjectRef::new(PyObject::BuiltinFunction {
        name: "__init_subclass__".to_string(),
        func: |_args| {
            Ok(py_none())
        },
    }));
    // __class_getitem__(cls, item): for generic types like List[int] (PEP 560)
    object_dict.insert_str("__class_getitem__", PyObjectRef::new(PyObject::BuiltinFunction {
        name: "__class_getitem__".to_string(),
        func: |args| {
            if args.len() < 2 {
                return Err(PyError::type_error("__class_getitem__ requires at least 2 arguments (cls, item)"));
            }
            Ok(py_tuple(vec![args[0].clone(), args[1].clone()]))
        },
    }));
    // __format__(self, format_spec): basic format support
    object_dict.insert_str("__format__", PyObjectRef::new(PyObject::BuiltinFunction {
        name: "__format__".to_string(),
        func: |args| {
            if args.is_empty() {
                return Err(PyError::type_error("__format__ requires at least 1 argument (self)"));
            }
            let obj = &args[0];
            let spec = if args.len() > 1 { args[1].str() } else { String::new() };
            if spec.is_empty() {
                Ok(py_str(&obj.repr()))
            } else {
                Err(PyError::value_error(format!("unknown format code '{}' for object", spec)))
            }
        },
    }));
    // __reduce__(self): basic pickle support
    object_dict.insert_str("__reduce__", PyObjectRef::new(PyObject::BuiltinFunction {
        name: "__reduce__".to_string(),
        func: |_args| {
            Ok(py_none())
        },
    }));
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
    builtins.insert_str("_object_func", PyObjectRef::new(PyObject::BuiltinFunction {
        name: "object".to_string(),
        func: builtin_object,
    }));

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
    int_dict.insert_str(crate::object::NATIVE_VALUE_CTOR_KEY, PyObjectRef::new(PyObject::BuiltinFunction {
        name: "int".to_string(),
        func: builtin_int,
    }));
    int_dict.insert_str("from_bytes", PyObjectRef::new(PyObject::BuiltinFunction {
        name: "from_bytes".to_string(),
        func: crate::object::builtin_int_from_bytes,
    }));
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
    str_dict.insert_str(crate::object::NATIVE_VALUE_CTOR_KEY, PyObjectRef::new(PyObject::BuiltinFunction {
        name: "str".to_string(),
        func: builtin_str,
    }));
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
    list_dict.insert_str(crate::object::NATIVE_VALUE_CTOR_KEY, PyObjectRef::new(PyObject::BuiltinFunction {
        name: "list".to_string(),
        func: builtin_list,
    }));
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
    float_dict.insert_str(crate::object::NATIVE_VALUE_CTOR_KEY, PyObjectRef::new(PyObject::BuiltinFunction {
        name: "float".to_string(),
        func: builtin_float,
    }));
    float_dict.insert_str("__getformat__", PyObjectRef::new(PyObject::BuiltinFunction {
        name: "__getformat__".to_string(),
        func: |_args| Ok(py_str("IEEE, little-endian")),
    }));
    float_dict.insert_str("fromhex", PyObjectRef::new(PyObject::BuiltinFunction {
        name: "fromhex".to_string(),
        func: crate::object::float_fromhex,
    }));
    float_dict.insert_str("hex", PyObjectRef::new(PyObject::BuiltinFunction {
        name: "hex".to_string(),
        func: crate::object::float_class_hex,
    }));
    float_dict.insert_str("from_number", PyObjectRef::new(PyObject::BuiltinFunction {
        name: "from_number".to_string(),
        func: |args| {
            if args.is_empty() { return Err(PyError::type_error("float.from_number() takes exactly 1 argument")); }
            Ok(py_float(args[0].as_f64().unwrap_or(f64::NAN)))
        },
    }));
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
    dict_dict.insert_str(crate::object::NATIVE_VALUE_CTOR_KEY, PyObjectRef::new(PyObject::BuiltinFunction {
        name: "dict".to_string(),
        func: builtin_dict,
    }));
    dict_dict.insert_str("fromkeys", PyObjectRef::new(PyObject::BuiltinFunction {
        name: "fromkeys".to_string(),
        func: |args| {
            if args.is_empty() { return Err(PyError::type_error("fromkeys() takes at least 1 argument")); }
            let keys = crate::object::collect_iterable(&args[0])?;
            let value = args.get(1).cloned().unwrap_or_else(py_none);
            let mut d = PyDict::new();
            for k in keys {
                d.set(k, value.clone())?;
            }
            Ok(PyObjectRef::new(PyObject::Dict(Box::new(d))))
        },
    }));
    dict_dict.insert_str("__setitem__", PyObjectRef::imm(PyObject::BuiltinMethod {
        name: "__setitem__".to_string(),
        func: crate::object::builtin_dict_setitem as BuiltinFunc,
        self_obj: py_none(),
    }));
    dict_dict.insert_str("__getitem__", PyObjectRef::imm(PyObject::BuiltinMethod {
        name: "__getitem__".to_string(),
        func: crate::object::builtin_dict_getitem as BuiltinFunc,
        self_obj: py_none(),
    }));
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
    tuple_dict.insert_str(crate::object::NATIVE_VALUE_CTOR_KEY, PyObjectRef::new(PyObject::BuiltinFunction {
        name: "tuple".to_string(),
        func: builtin_tuple,
    }));
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
    bytes_dict.insert_str(crate::object::NATIVE_VALUE_CTOR_KEY, PyObjectRef::new(PyObject::BuiltinFunction {
        name: "bytes".to_string(),
        func: builtin_bytes,
    }));
    bytes_dict.insert_str("fromhex", PyObjectRef::new(PyObject::BuiltinFunction {
        name: "fromhex".to_string(),
        func: builtin_bytes_fromhex,
    }));
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
    set_dict.insert_str(crate::object::NATIVE_VALUE_CTOR_KEY, PyObjectRef::new(PyObject::BuiltinFunction {
        name: "set".to_string(),
        func: builtin_set,
    }));
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
    complex_dict.insert_str(crate::object::NATIVE_VALUE_CTOR_KEY, PyObjectRef::new(PyObject::BuiltinFunction {
        name: "complex".to_string(),
        func: builtin_complex,
    }));
    complex_dict.insert_str("from_number", PyObjectRef::new(PyObject::BuiltinFunction {
        name: "from_number".to_string(),
        func: |args| {
            if args.is_empty() { return Err(PyError::type_error("complex.from_number() takes exactly 1 argument")); }
            let n = args[0].as_f64().unwrap_or(0.0);
            Ok(PyObjectRef::imm(PyObject::Complex(n, 0.0)))
        },
    }));
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
    bytearray_dict.insert_str(crate::object::NATIVE_VALUE_CTOR_KEY, PyObjectRef::new(PyObject::BuiltinFunction {
        name: "bytearray".to_string(),
        func: builtin_bytearray,
    }));
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
    frozenset_dict.insert_str(crate::object::NATIVE_VALUE_CTOR_KEY, PyObjectRef::new(PyObject::BuiltinFunction {
        name: "frozenset".to_string(),
        func: builtin_frozenset,
    }));
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
    bool_dict.insert_str(crate::object::NATIVE_VALUE_CTOR_KEY, PyObjectRef::new(PyObject::BuiltinFunction {
        name: "bool".to_string(),
        func: builtin_bool,
    }));
    bool_dict.insert_str("__new__", PyObjectRef::new(PyObject::BuiltinFunction {
        name: "__new__".to_string(),
        func: |args| {
            if args.is_empty() { return Ok(py_bool(false)); }
            if args.len() >= 2 { return Ok(py_bool(args[1].truthy())); }
            Ok(py_bool(false))
        },
    }));
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
    type_dict.insert_str("__new__", PyObjectRef::new(PyObject::StaticMethod {
        func: PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__new__".to_string(),
            func: crate::object::type_new_builtin,
        }),
    }));
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
    builtins.insert_str("_type_func", PyObjectRef::new(PyObject::BuiltinFunction {
        name: "type".to_string(),
        func: builtin_type_of,
    }));

    builtins
}

// ── _codecs builtin module helpers ──────────────────────────────────────────

/// Encode a string as UTF-8/ASCII/Latin-1 (used by codecs.lookup() results).
fn _codecs_encode(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 1 {
        return Err(PyError::type_error("encode() requires at least 1 argument"));
    }
    let s = args[0].str();
    let len = s.len();
    Ok(PyObjectRef::new(PyObject::Tuple(vec![
        PyObjectRef::imm(PyObject::Bytes(s.into_bytes())),
        py_int(len as i64),
    ])))
}

/// Decode bytes as UTF-8 (used by codecs.lookup() results).
fn _codecs_decode(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 1 {
        return Err(PyError::type_error("decode() requires at least 1 argument"));
    }
    let data = {
        let obj = args[0].borrow();
        match &*obj {
            PyObject::Bytes(b) => b.clone(),
            _ => return Err(PyError::type_error("decode() argument must be bytes")),
        }
    };
    let s = String::from_utf8(data)
        .map_err(|e| PyError::value_error(format!("decode error: {}", e)))?;
    let len = s.len();
    Ok(PyObjectRef::new(PyObject::Tuple(vec![
        py_str(&s),
        py_int(len as i64),
    ])))
}

fn _codecs_reader(_args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    Err(PyError::value_error("stream reader not implemented"))
}

fn _codecs_writer(_args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    Err(PyError::value_error("stream writer not implemented"))
}

fn _codecs_lookup_error(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 1 {
        return Err(PyError::type_error("lookup_error() requires at least 1 argument"));
    }
    let name = args[0].str().to_lowercase();
    match name.as_str() {
        "strict" | "ignore" | "replace"
        | "xmlcharrefreplace" | "backslashreplace"
        | "namereplace" | "surrogateescape" | "surrogatepass" => {
            Ok(py_str(&name))
        }
        _ => Err(PyError::value_error(format!(
            "unknown error handler: '{}'", name
        ))),
    }
}

fn _codecs_lookup(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 1 {
        return Err(PyError::type_error("lookup() requires at least 1 argument"));
    }
    let encoding = args[0].str().to_lowercase().replace('-', "_");
    match encoding.as_str() {
        "utf_8" | "ascii" | "latin_1" | "utf8" => {
            Ok(PyObjectRef::new(PyObject::Tuple(vec![
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: "encode".to_string(),
                    func: _codecs_encode,
                }),
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: "decode".to_string(),
                    func: _codecs_decode,
                }),
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: "stream_reader".to_string(),
                    func: _codecs_reader,
                }),
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: "stream_writer".to_string(),
                    func: _codecs_writer,
                }),
                py_str(&encoding),
            ])))
        }
        _ => Err(PyError::value_error(format!(
            "unknown encoding: {}", encoding
        ))),
    }
}

fn _codecs_encode_func(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Err(PyError::type_error("encode() requires at least 2 arguments"));
    }
    let s = args[0].str();
    let encoding = args[1].str().to_lowercase().replace('-', "_");
    match encoding.as_str() {
        "utf_8" | "ascii" | "latin_1" | "utf8" => {
            let len = s.len();
            Ok(PyObjectRef::new(PyObject::Tuple(vec![
                PyObjectRef::imm(PyObject::Bytes(s.into_bytes())),
                py_int(len as i64),
            ])))
        }
        _ => Err(PyError::value_error(format!(
            "unknown encoding: {}", encoding
        ))),
    }
}

fn _codecs_decode_func(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Err(PyError::type_error("decode() requires at least 2 arguments"));
    }
    let data = {
        let obj = args[0].borrow();
        match &*obj {
            PyObject::Bytes(b) => b.clone(),
            _ => return Err(PyError::type_error("decode() argument must be bytes")),
        }
    };
    let encoding = args[1].str().to_lowercase().replace('-', "_");
    match encoding.as_str() {
        "utf_8" | "ascii" | "latin_1" | "utf8" => {
            let s = String::from_utf8(data)
                .map_err(|e| PyError::value_error(format!("decode error: {}", e)))?;
            let len = s.len();
            Ok(PyObjectRef::new(PyObject::Tuple(vec![
                py_str(&s),
                py_int(len as i64),
            ])))
        }
        _ => Err(PyError::value_error(format!(
            "unknown encoding: {}", encoding
        ))),
    }
}

/// Create the `_codecs` module dictionary.
pub fn create_codecs_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    d.insert_str("lookup_error", PyObjectRef::new(PyObject::BuiltinFunction {
        name: "lookup_error".to_string(),
        func: _codecs_lookup_error,
    }));
    d.insert_str("lookup", PyObjectRef::new(PyObject::BuiltinFunction {
        name: "lookup".to_string(),
        func: _codecs_lookup,
    }));
    d.insert_str("encode", PyObjectRef::new(PyObject::BuiltinFunction {
        name: "encode".to_string(),
        func: _codecs_encode_func,
    }));
    d.insert_str("decode", PyObjectRef::new(PyObject::BuiltinFunction {
        name: "decode".to_string(),
        func: _codecs_decode_func,
    }));
    d.insert_str("register", PyObjectRef::new(PyObject::BuiltinFunction {
        name: "register".to_string(),
        func: |args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
            if args.len() < 1 {
                return Err(PyError::type_error("register() requires at least 1 argument"));
            }
            CODEC_SEARCH_FUNCTIONS.with(|fns| {
                fns.borrow_mut().push(args[0].clone());
            });
            Ok(py_none())
        },
    }));
    d.insert_str("unregister", PyObjectRef::new(PyObject::BuiltinFunction {
        name: "unregister".to_string(),
        func: |args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
            if args.len() < 1 {
                return Err(PyError::type_error("unregister() requires at least 1 argument"));
            }
            CODEC_SEARCH_FUNCTIONS.with(|fns| {
                fns.borrow_mut().retain(|f| !f.is(&args[0]));
            });
            Ok(py_none())
        },
    }));
    d
}

// ── _abc builtin module helpers ──────────────────────────────────────────
// Needed by `abc.py` (stdlib) for Abstract Base Class support.
// In CPython, `_abc` is a C extension. We provide the same API in Rust
// so that `from _abc import (ABCMeta, get_cache_token, ...)` succeeds,
// which is required for `io/__init__.py` → `import abc` during Django setup.

/// Global invalidation counter for ABC virtual-subclass caches.
static ABC_CACHE_TOKEN: AtomicI64 = AtomicI64::new(0);

fn _abc_get_cache_token(_args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    Ok(py_int(ABC_CACHE_TOKEN.load(std::sync::atomic::Ordering::Relaxed)))
}

fn _abc_init(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 1 {
        return Err(PyError::type_error("_abc_init() requires at least 1 argument"));
    }
    let cls = &args[0];
    // Set _abc_impl if not already present (computed from bases)
    let needs_impl = {
        let b = cls.borrow();
        match &*b {
            PyObject::Type { dict, .. } => !dict.contains_key_str("_abc_impl"),
            _ => return Err(PyError::type_error("_abc_init() argument must be a type")),
        }
    };
    if needs_impl {
        let bases: Vec<PyObjectRef> = {
            let b = cls.borrow();
            match &*b {
                PyObject::Type { bases, .. } => bases.clone(),
                _ => vec![],
            }
        };
        let mut impl_set = PySet::new();
        for base in &bases {
            // Each ABC base contributes its _abc_impl (or itself)
            if let Ok(abc_impl) = base.borrow().get_attribute("_abc_impl") {
                if let PyObject::FrozenSet(items) = &*abc_impl.borrow() {
                    for item in items.to_vec() {
                        impl_set.add(item)?;
                    }
                }
            }
            // Also add base itself if it's an instance of ABCMeta (has _abc_impl)
            if base.borrow().get_attribute("_abc_impl").is_ok() {
                impl_set.add(base.clone())?;
            }
        }
        cls.borrow_mut().set_attribute("_abc_impl",
            PyObjectRef::imm(PyObject::FrozenSet(impl_set)))?;
    }
    // Ensure standard ABC attributes exist
    for attr in &["_abc_registry", "_abc_cache", "_abc_negative_cache"] {
        let has = cls.borrow().get_attribute(attr).is_ok();
        if !has {
            cls.borrow_mut().set_attribute(attr, py_set())?;
        }
    }
    let has_ver = cls.borrow().get_attribute("_abc_negative_cache_version").is_ok();
    if !has_ver {
        cls.borrow_mut().set_attribute("_abc_negative_cache_version", py_int(0))?;
    }
    Ok(py_none())
}

fn _abc_register(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Err(PyError::type_error("_abc_register() requires at least 2 arguments"));
    }
    let cls = &args[0];
    let subclass = &args[1].clone();
    // Ensure registry exists — use a FrozenSet
    if cls.borrow().get_attribute("_abc_registry").is_err() {
        cls.borrow_mut().set_attribute("_abc_registry",
            PyObjectRef::imm(PyObject::FrozenSet(PySet::new())))?;
    }
    // Get current registry, add subclass if not already present
    let mut registered = {
        let r = cls.borrow().get_attribute("_abc_registry")?;
        let b = r.borrow();
        match &*b {
            PyObject::FrozenSet(items) => items.to_vec(),
            _ => vec![],
        }
    };
    if !registered.iter().any(|r| r.is(subclass)) {
        registered.push(subclass.clone());
    }
    // Build PySet from registered Vec
    let mut reg_set = PySet::new();
    for item in &registered { reg_set.add(item.clone())?; }
    cls.borrow_mut().set_attribute("_abc_registry",
        PyObjectRef::imm(PyObject::FrozenSet(reg_set)))?;
    // Invalidate cache
    ABC_CACHE_TOKEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    Ok(args[1].clone())
}

fn _abc_instancecheck(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    // Stub: fall back to normal Python isinstance
    if args.len() < 2 {
        return Err(PyError::type_error("_abc_instancecheck() requires at least 2 arguments"));
    }
    Ok(py_bool(false))
}

fn _abc_subclasscheck(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    // Stub: fall back to normal Python issubclass
    if args.len() < 2 {
        return Err(PyError::type_error("_abc_subclasscheck() requires at least 2 arguments"));
    }
    Ok(py_bool(false))
}

fn _abc_get_dump(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 1 {
        return Err(PyError::type_error("_get_dump() requires at least 1 argument"));
    }
    let cls = &args[0];
    let registry = cls.borrow().get_attribute("_abc_registry").unwrap_or_else(|_| py_dict());
    let cache = cls.borrow().get_attribute("_abc_cache").unwrap_or_else(|_| py_dict());
    let neg_cache = cls.borrow().get_attribute("_abc_negative_cache").unwrap_or_else(|_| py_dict());
    let version = cls.borrow().get_attribute("_abc_negative_cache_version").unwrap_or(py_int(0));
    Ok(PyObjectRef::new(PyObject::Tuple(vec![registry, cache, neg_cache, version])))
}

fn _abc_reset_registry(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 1 {
        return Err(PyError::type_error("_reset_registry() requires at least 1 argument"));
    }
    let cls = &args[0];
    cls.borrow_mut().set_attribute("_abc_registry", py_set())?;
    Ok(py_none())
}

fn _abc_reset_caches(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 1 {
        return Err(PyError::type_error("_reset_caches() requires at least 1 argument"));
    }
    let cls = &args[0];
    cls.borrow_mut().set_attribute("_abc_cache", py_set())?;
    cls.borrow_mut().set_attribute("_abc_negative_cache", py_set())?;
    Ok(py_none())
}

/// ABCMeta(name, bases, namespace) -> new class with ABC support.
///
/// This is the metaclass that `abc.ABCMeta` inherits from `type` in CPython.
/// Since our RustPython doesn't have a full C-level `type` metaclass with
/// 3-argument form, we expose a builtin function that creates a class and
/// calls `_abc_init` to set up ABC data structures.
fn _abc_abcmeta(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 3 {
        return Err(PyError::type_error("ABCMeta() requires at least 3 arguments"));
    }
    let name_str = args[0].str();
    let bases_vec = if let PyObject::Tuple(t) = &*args[1].borrow() {
        t.clone()
    } else {
        return Err(PyError::type_error("ABCMeta() bases must be a tuple"));
    };
    let namespace_dict = {
        let b = args[2].borrow();
        if let PyObject::Dict(d) = &*b {
            let mut h = HashMap::new();
            for (k, v) in d.items() {
                h.insert(k.str(), v);
            }
            h
        } else {
            return Err(PyError::type_error("ABCMeta() namespace must be a dict"));
        }
    };
    let class = PyObjectRef::new(PyObject::Type {
        name: name_str,
        dict: Box::new(str_map_to_typedict(namespace_dict)),
        bases: bases_vec.clone(),
        mro: vec![],
    });
    // Compute and set MRO
    let mut mro = vec![class.clone()];
    for base in &bases_vec {
        mro.push(base.clone());
    }
    if let PyObject::Type { mro: mro_field, .. } = &mut *class.borrow_mut() {
        *mro_field = mro;
    }
    // Run _abc_init to set up ABC data structures
    let _ = _abc_init(&[class.clone()]);
    Ok(class)
}

/// Create the `_abc` module dictionary.
pub fn create_abc_builtins_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    d.insert_str("ABCMeta", PyObjectRef::new(PyObject::BuiltinFunction {
        name: "ABCMeta".to_string(),
        func: _abc_abcmeta,
    }));
    d.insert_str("get_cache_token", PyObjectRef::new(PyObject::BuiltinFunction {
        name: "get_cache_token".to_string(),
        func: _abc_get_cache_token,
    }));
    d.insert_str("_abc_init", PyObjectRef::new(PyObject::BuiltinFunction {
        name: "_abc_init".to_string(),
        func: _abc_init,
    }));
    d.insert_str("_abc_register", PyObjectRef::new(PyObject::BuiltinFunction {
        name: "_abc_register".to_string(),
        func: _abc_register,
    }));
    d.insert_str("_abc_instancecheck", PyObjectRef::new(PyObject::BuiltinFunction {
        name: "_abc_instancecheck".to_string(),
        func: _abc_instancecheck,
    }));
    d.insert_str("_abc_subclasscheck", PyObjectRef::new(PyObject::BuiltinFunction {
        name: "_abc_subclasscheck".to_string(),
        func: _abc_subclasscheck,
    }));
    d.insert_str("_get_dump", PyObjectRef::new(PyObject::BuiltinFunction {
        name: "_get_dump".to_string(),
        func: _abc_get_dump,
    }));
    d.insert_str("_reset_registry", PyObjectRef::new(PyObject::BuiltinFunction {
        name: "_reset_registry".to_string(),
        func: _abc_reset_registry,
    }));
    d.insert_str("_reset_caches", PyObjectRef::new(PyObject::BuiltinFunction {
        name: "_reset_caches".to_string(),
        func: _abc_reset_caches,
    }));
    d
}

/// Like `PyObjectRef::as_f64()`, but also consults `__float__` for an
/// `Instance` that isn't a native numeric type — real Python's `math`
/// functions all accept ANY object implementing `__float__` (e.g. custom
/// numeric-like classes, `decimal.Decimal`, `fractions.Fraction`), not just
/// literal `int`/`float`. Most of `math`'s own native functions previously
/// used bare `.as_f64()` directly, which only ever handles native
/// int/float/bool — rejecting a perfectly valid `__float__`-defining object
/// with a spurious `TypeError`. Found via CPython's own `test_math.py`
/// (`hypot(0.75, FloatLike(-1.))` and similar for `isclose`/`isnan`/
/// `copysign`/`fmod`/`atan2`/`dist`/`sumprod`).
fn math_arg_f64(v: &PyObjectRef) -> Option<f64> {
    if let Some(f) = v.as_f64() { return Some(f); }
    let f = {
        let typ = if let PyObject::Instance { typ, .. } = &*v.borrow() { Some(typ.clone()) } else { None }?;
        lookup_dunder_via_mro(&typ, "__float__")?
    };
    call_bound_method(f, v.clone(), vec![]).ok()?.as_f64()
}

pub fn create_math_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! math_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }
    math_func!("sqrt", |args| {
        if args.len() != 1 { return Err(PyError::type_error("sqrt() takes exactly one argument")); }
        let v = args[0].borrow();
        match &*v { PyObject::Int(i) => Ok(py_float(i.to_f64().unwrap_or(0.0).sqrt())), PyObject::Float(f) => Ok(py_float(f.sqrt())), _ => Err(PyError::type_error("sqrt() argument must be a number")) }
    });
    math_func!("sin", |args| {
        if args.len() != 1 { return Err(PyError::type_error("sin() takes exactly one argument")); }
        let v = args[0].borrow();
        match &*v { PyObject::Int(i) => Ok(py_float(i.to_f64().unwrap_or(0.0).sin())), PyObject::Float(f) => Ok(py_float(f.sin())), _ => Err(PyError::type_error("sin() argument must be a number")) }
    });
    math_func!("cos", |args| {
        if args.len() != 1 { return Err(PyError::type_error("cos() takes exactly one argument")); }
        let v = args[0].borrow();
        match &*v { PyObject::Int(i) => Ok(py_float(i.to_f64().unwrap_or(0.0).cos())), PyObject::Float(f) => Ok(py_float(f.cos())), _ => Err(PyError::type_error("cos() argument must be a number")) }
    });
    math_func!("tan", |args| {
        if args.len() != 1 { return Err(PyError::type_error("tan() takes exactly one argument")); }
        let v = args[0].borrow();
        match &*v { PyObject::Int(i) => Ok(py_float(i.to_f64().unwrap_or(0.0).tan())), PyObject::Float(f) => Ok(py_float(f.tan())), _ => Err(PyError::type_error("tan() argument must be a number")) }
    });
    math_func!("floor", |args| {
        if args.len() != 1 { return Err(PyError::type_error("floor() takes exactly one argument")); }
        let v = args[0].borrow();
        match &*v { PyObject::Int(i) => Ok(py_int(i.clone())), PyObject::Float(f) => Ok(py_int(f.floor() as i64)), _ => Err(PyError::type_error("floor() argument must be a number")) }
    });
    math_func!("ceil", |args| {
        if args.len() != 1 { return Err(PyError::type_error("ceil() takes exactly one argument")); }
        let v = args[0].borrow();
        match &*v { PyObject::Int(i) => Ok(py_int(i.clone())), PyObject::Float(f) => Ok(py_int(f.ceil() as i64)), _ => Err(PyError::type_error("ceil() argument must be a number")) }
    });
    math_func!("exp", |args| {
        if args.len() != 1 { return Err(PyError::type_error("exp() takes exactly one argument")); }
        let v = args[0].borrow();
        match &*v { PyObject::Int(i) => Ok(py_float(i.to_f64().unwrap_or(0.0).exp())), PyObject::Float(f) => Ok(py_float(f.exp())), _ => Err(PyError::type_error("exp() argument must be a number")) }
    });
    math_func!("pow", |args| {
        if args.len() != 2 { return Err(PyError::type_error("pow() takes exactly two arguments")); }
        let a = args[0].borrow();
        let b = args[1].borrow();
        let (x, y) = match (&*a, &*b) {
            (PyObject::Int(i), PyObject::Int(j)) => (i.to_f64().unwrap_or(0.0), j.to_f64().unwrap_or(0.0)),
            (PyObject::Int(i), PyObject::Float(f)) => (i.to_f64().unwrap_or(0.0), *f),
            (PyObject::Float(f), PyObject::Int(i)) => (*f, i.to_f64().unwrap_or(0.0)),
            (PyObject::Float(a), PyObject::Float(b)) => (*a, *b),
            _ => return Err(PyError::type_error("pow() argument must be a number")),
        };
        // `0 ** negative` is a real domain error (division by zero), not a
        // silent `inf`/`nan` — matches real CPython's own `math.pow`
        // (`ValueError: math domain error`).
        // Only a FINITE negative exponent is a domain error — `0.0 **
        // -inf` legitimately diverges to `inf` (matches the underlying
        // C `pow()` and real CPython's own `math.pow(0., NINF) == INF`).
        if x == 0.0 && y < 0.0 && y.is_finite() {
            return Err(PyError::value_error("math domain error"));
        }
        // A negative base raised to a finite, non-integer exponent has no
        // real result (it's genuinely complex) — real CPython's `math.pow`
        // raises `ValueError: math domain error` here too, rather than the
        // `NaN` plain `f64::powf` produces.
        if x < 0.0 && x.is_finite() && y.is_finite() && y.fract() != 0.0 {
            return Err(PyError::value_error("math domain error"));
        }
        let result = x.powf(y);
        // A genuine overflow (both inputs finite, result isn't) must raise
        // `OverflowError`, not silently return `inf` — legitimate infinite
        // results (`pow(INF, 1)`, `pow(x, INF)`, etc.) are unaffected since
        // at least one input is already infinite in those cases. Found via
        // CPython's own `test_math.py::testPow` (`math.pow(1e+100,
        // 1e+100)`).
        if result.is_infinite() && x.is_finite() && y.is_finite() {
            return Err(PyError::overflow_error("math range error"));
        }
        Ok(py_float(result))
    });
    math_func!("fma", |args| {
        if args.len() != 3 { return Err(PyError::type_error("fma() takes exactly three arguments")); }
        let a = args[0].as_f64().unwrap_or(f64::NAN);
        let b = args[1].as_f64().unwrap_or(f64::NAN);
        let c = args[2].as_f64().unwrap_or(f64::NAN);
        Ok(py_float(a.mul_add(b, c)))
    });
    math_func!("log", |args| {
        if args.len() < 1 || args.len() > 2 { return Err(PyError::type_error("log() takes one or two arguments")); }
        let v = args[0].borrow();
        let x = match &*v { PyObject::Int(i) => i.to_f64().unwrap_or(0.0), PyObject::Float(f) => *f, _ => return Err(PyError::type_error("log() argument must be a number")) };
        let base = if args.len() == 2 {
            let b = args[1].borrow();
            match &*b { PyObject::Int(i) => i.to_f64().unwrap_or(0.0), PyObject::Float(f) => *f, _ => return Err(PyError::type_error("log() base must be a number")) }
        } else {
            std::f64::consts::E
        };
        Ok(py_float(x.log(base)))
    });
    math_func!("abs", |args| {
        if args.len() != 1 { return Err(PyError::type_error("abs() takes exactly one argument")); }
        let v = args[0].borrow();
        match &*v { PyObject::Int(i) => Ok(py_float(i.to_f64().unwrap_or(0.0).abs())), PyObject::Float(f) => Ok(py_float(f.abs())), _ => Err(PyError::type_error("abs() argument must be a number")) }
    });
    // ── New math functions needed by CPython's random.py ──────────────────
    math_func!("acos", |args| {
        if args.len() != 1 { return Err(PyError::type_error("acos() takes exactly one argument")); }
        let v = args[0].borrow();
        match &*v { PyObject::Int(i) => Ok(py_float(i.to_f64().unwrap_or(0.0).acos())), PyObject::Float(f) => Ok(py_float(f.acos())), _ => Err(PyError::type_error("acos() argument must be a number")) }
    });
    math_func!("fabs", |args| {
        if args.len() != 1 { return Err(PyError::type_error("fabs() takes exactly one argument")); }
        let v = args[0].borrow();
        match &*v { PyObject::Int(i) => Ok(py_float(i.to_f64().unwrap_or(0.0).abs())), PyObject::Float(f) => Ok(py_float(f.abs())), _ => Err(PyError::type_error("fabs() argument must be a number")) }
    });
    math_func!("isfinite", |args| {
        if args.len() != 1 { return Err(PyError::type_error("isfinite() takes exactly one argument")); }
        let v = args[0].borrow();
        match &*v { PyObject::Int(_) => Ok(py_bool(true)), PyObject::Float(f) => Ok(py_bool(f.is_finite())), _ => Err(PyError::type_error("isfinite() argument must be a number")) }
    });
    math_func!("lgamma", |args| {
        if args.len() != 1 { return Err(PyError::type_error("lgamma() takes exactly one argument")); }
        let v = args[0].borrow();
        match &*v { PyObject::Int(i) => Ok(py_float(libm::lgamma(i.to_f64().unwrap_or(0.0)))), PyObject::Float(f) => Ok(py_float(libm::lgamma(*f))), _ => Err(PyError::type_error("lgamma() argument must be a number")) }
    });
    math_func!("log2", |args| {
        if args.len() != 1 { return Err(PyError::type_error("log2() takes exactly one argument")); }
        let v = args[0].borrow();
        match &*v { PyObject::Int(i) => Ok(py_float(i.to_f64().unwrap_or(0.0).log2())), PyObject::Float(f) => Ok(py_float(f.log2())), _ => Err(PyError::type_error("log2() argument must be a number")) }
    });
    // ── More single-float-argument math functions ──────────────────────────
    // `math` was missing most of its real surface (only 15 functions total
    // before this) — real code reaches for any of these routinely (Django's
    // own sqlite3 backend registers several as SQL functions: `asin`, real
    // trigger for this batch). Using `as_f64()`/`py_float` directly instead
    // of each one's own hand-written Int/Float match arms, since `as_f64()`
    // already handles both.
    macro_rules! math_func1 {
        ($name:expr, $f:expr) => {
            math_func!($name, |args| {
                if args.len() != 1 { return Err(PyError::type_error(concat!($name, "() takes exactly one argument"))); }
                let x = math_arg_f64(&args[0]).ok_or_else(|| PyError::type_error(concat!($name, "() argument must be a number")))?;
                Ok(py_float(($f)(x)))
            });
        };
    }
    math_func1!("asin", f64::asin);
    math_func1!("atan", f64::atan);
    math_func1!("sinh", f64::sinh);
    math_func1!("cosh", f64::cosh);
    math_func1!("tanh", f64::tanh);
    math_func1!("asinh", f64::asinh);
    math_func1!("acosh", f64::acosh);
    math_func1!("atanh", f64::atanh);
    math_func1!("degrees", f64::to_degrees);
    math_func1!("radians", f64::to_radians);
    math_func1!("log10", f64::log10);
    math_func1!("log1p", f64::ln_1p);
    math_func1!("expm1", f64::exp_m1);
    math_func1!("trunc", f64::trunc);
    math_func1!("cbrt", f64::cbrt);
    math_func1!("exp2", f64::exp2);
    math_func1!("erf", libm::erf);
    math_func1!("erfc", libm::erfc);
    math_func1!("gamma", libm::tgamma);

    math_func!("atan2", |args| {
        if args.len() != 2 { return Err(PyError::type_error("atan2() takes exactly two arguments")); }
        let y = math_arg_f64(&args[0]).ok_or_else(|| PyError::type_error("atan2() argument must be a number"))?;
        let x = math_arg_f64(&args[1]).ok_or_else(|| PyError::type_error("atan2() argument must be a number"))?;
        Ok(py_float(y.atan2(x)))
    });
    math_func!("hypot", |args| {
        let mut sum_sq = 0.0f64;
        for a in args {
            let v = math_arg_f64(&a).ok_or_else(|| PyError::type_error("hypot() arguments must be numbers"))?;
            sum_sq += v * v;
        }
        Ok(py_float(sum_sq.sqrt()))
    });
    math_func!("copysign", |args| {
        if args.len() != 2 { return Err(PyError::type_error("copysign() takes exactly two arguments")); }
        let x = math_arg_f64(&args[0]).ok_or_else(|| PyError::type_error("copysign() argument must be a number"))?;
        let y = math_arg_f64(&args[1]).ok_or_else(|| PyError::type_error("copysign() argument must be a number"))?;
        Ok(py_float(x.copysign(y)))
    });
    math_func!("fmod", |args| {
        if args.len() != 2 { return Err(PyError::type_error("fmod() takes exactly two arguments")); }
        let x = math_arg_f64(&args[0]).ok_or_else(|| PyError::type_error("fmod() argument must be a number"))?;
        let y = math_arg_f64(&args[1]).ok_or_else(|| PyError::type_error("fmod() argument must be a number"))?;
        Ok(py_float(x % y))
    });
    math_func!("isnan", |args| {
        if args.len() != 1 { return Err(PyError::type_error("isnan() takes exactly one argument")); }
        let x = math_arg_f64(&args[0]).ok_or_else(|| PyError::type_error("isnan() argument must be a number"))?;
        Ok(py_bool(x.is_nan()))
    });
    math_func!("isinf", |args| {
        if args.len() != 1 { return Err(PyError::type_error("isinf() takes exactly one argument")); }
        let x = math_arg_f64(&args[0]).ok_or_else(|| PyError::type_error("isinf() argument must be a number"))?;
        Ok(py_bool(x.is_infinite()))
    });
    math_func!("isclose", |args| {
        if args.len() < 2 { return Err(PyError::type_error("isclose() takes at least two arguments")); }
        let a = math_arg_f64(&args[0]).ok_or_else(|| PyError::type_error("isclose() argument must be a number"))?;
        let b = math_arg_f64(&args[1]).ok_or_else(|| PyError::type_error("isclose() argument must be a number"))?;
        // `rel_tol`/`abs_tol` (real `math.isclose`'s signature: `isclose(a,
        // b, *, rel_tol=1e-09, abs_tol=0.0)`, keyword-only) were hardcoded
        // to their defaults, completely ignoring whatever the caller
        // actually passed — `math.isclose(1.0, 1.0000001, rel_tol=1e-5)`
        // silently used `1e-9` instead, returning `False` for a
        // comparison that should clearly be `True`. Keyword args arrive
        // packed into a trailing dict per this codebase's own
        // `BuiltinFunction` calling convention.
        let mut rel_tol = 1e-9;
        let mut abs_tol = 0.0;
        if let Some(last) = args.last() {
            if let PyObject::Dict(kwargs) = &*last.borrow() {
                if let Ok(Some(v)) = kwargs.get(&py_str("rel_tol")) {
                    rel_tol = math_arg_f64(&v).ok_or_else(|| PyError::type_error("isclose() argument must be a number"))?;
                }
                if let Ok(Some(v)) = kwargs.get(&py_str("abs_tol")) {
                    abs_tol = math_arg_f64(&v).ok_or_else(|| PyError::type_error("isclose() argument must be a number"))?;
                }
            }
        }
        if rel_tol < 0.0 || abs_tol < 0.0 {
            return Err(PyError::value_error("tolerances must be non-negative"));
        }
        if a == b { return Ok(py_bool(true)); }
        if a.is_infinite() || b.is_infinite() { return Ok(py_bool(false)); }
        Ok(py_bool((a - b).abs() <= (rel_tol * a.abs().max(b.abs())).max(abs_tol)))
    });
    math_func!("gcd", |args| {
        fn gcd(a: i64, b: i64) -> i64 { if b == 0 { a.abs() } else { gcd(b, a % b) } }
        let mut result = 0i64;
        for a in args {
            let v = a.as_i64().ok_or_else(|| PyError::type_error("gcd() arguments must be integers"))?;
            result = gcd(result, v);
        }
        Ok(py_int(result))
    });
    math_func!("factorial", |args| {
        if args.len() != 1 { return Err(PyError::type_error("factorial() takes exactly one argument")); }
        let n = args[0].as_i64().ok_or_else(|| PyError::type_error("factorial() argument must be an integer"))?;
        if n < 0 { return Err(PyError::value_error("factorial() not defined for negative values")); }
        let mut result = num_bigint::BigInt::from(1i64);
        for i in 2..=n {
            result *= num_bigint::BigInt::from(i);
        }
        Ok(py_int(result))
    });
    // `math.isqrt` was missing entirely (not even a stub) — real trigger:
    // CPython's own `test_math.testIsqrt`, which feeds it values up to
    // `2**200` and `10**5001`. Since those are real arbitrary-precision
    // bigints, this MUST use a proper bigint square root (`num_bigint`'s own
    // `BigInt::sqrt`, a Newton's-method implementation) rather than
    // converting to `f64` first (`f64::sqrt` silently loses precision far
    // below even `2**64`, and can't represent a 5001-digit input at all).
    math_func!("isqrt", |args| {
        if args.len() != 1 { return Err(PyError::type_error("isqrt() takes exactly one argument")); }
        let n = match &*args[0].borrow() {
            PyObject::Int(i) => i.clone(),
            PyObject::Bool(b) => num_bigint::BigInt::from(if *b { 1 } else { 0 }),
            _ => return Err(PyError::type_error("isqrt() argument must be an integer")),
        };
        if n.sign() == num_bigint::Sign::Minus {
            return Err(PyError::value_error("isqrt() argument must be nonnegative"));
        }
        Ok(py_int(n.sqrt()))
    });

    // Additional math functions
    math_func!("ldexp", |args| {
        if args.len() < 2 { return Err(PyError::type_error("ldexp() requires 2 arguments")); }
        let x = math_arg_f64(&args[0]).ok_or_else(|| PyError::type_error("argument must be a number"))?;
        let exp = args[1].as_i64().ok_or_else(|| PyError::type_error("exponent must be an integer"))?;
        Ok(py_float(x * (2.0_f64).powi(exp as i32)))
    });
    math_func!("fsum", |args| {
        if args.is_empty() { return Err(PyError::type_error("fsum() requires an argument")); }
        // Previously only List/Tuple were handled directly — ANY other
        // iterable (a generator, `map()`/`filter()` object, custom
        // `__iter__`, ...) matched neither branch and silently returned
        // 0.0 WITHOUT ever iterating it at all. Real trigger: CPython's
        // own `Lib/statistics.py`, `fsum(map(log, count_positive(data)))`.
        // Fixed via the standard `collect_iterable` materialization (same
        // general fix already applied elsewhere for `set()`/`tuple()`).
        let items = collect_iterable(&args[0])?;
        let mut total = 0.0_f64;
        for item in &items {
            total += math_arg_f64(&item).ok_or_else(|| PyError::type_error(format!("must be real number, not {}", item.borrow().type_name())))?;
        }
        Ok(py_float(total))
    });
    // sumprod(p, q) — dot product of two equal-length iterables (added to
    // CPython 3.12). Needed by real CPython's own `Lib/statistics.py`
    // (`_sum`/`variance`'s exact-precision path). Simple f64 accumulation,
    // not CPython's exact-precision (Neumaier-summation) implementation —
    // matches this file's existing `fsum`'s own "simple summation" quality
    // bar rather than a from-scratch arbitrary-precision reimplementation.
    math_func!("sumprod", |args| {
        if args.len() < 2 { return Err(PyError::type_error("sumprod() requires 2 arguments")); }
        let p = collect_iterable(&args[0])?;
        let q = collect_iterable(&args[1])?;
        if p.len() != q.len() {
            return Err(PyError::value_error("inputs are not the same length"));
        }
        let mut total = 0.0_f64;
        for (a, b) in p.iter().zip(q.iter()) {
            let av = math_arg_f64(&a).ok_or_else(|| PyError::type_error("sumprod() arguments must be numbers"))?;
            let bv = math_arg_f64(&b).ok_or_else(|| PyError::type_error("sumprod() arguments must be numbers"))?;
            total += av * bv;
        }
        Ok(py_float(total))
    });
    math_func!("remainder", |args| {
        if args.len() < 2 { return Err(PyError::type_error("remainder() requires 2 arguments")); }
        let x = math_arg_f64(&args[0]).ok_or_else(|| PyError::type_error("argument must be a number"))?;
        let y = math_arg_f64(&args[1]).ok_or_else(|| PyError::type_error("argument must be a number"))?;
        Ok(py_float(x - y * (x / y).round()))
    });
    math_func!("modf", |args| {
        if args.is_empty() { return Err(PyError::type_error("modf() requires an argument")); }
        let x = math_arg_f64(&args[0]).ok_or_else(|| PyError::type_error("argument must be a number"))?;
        let frac = x.fract();
        let integer = x.trunc();
        Ok(py_tuple(vec![py_float(frac), py_float(integer)]))
    });
    math_func!("frexp", |args| {
        if args.is_empty() { return Err(PyError::type_error("frexp() requires an argument")); }
        let x = math_arg_f64(&args[0]).ok_or_else(|| PyError::type_error("argument must be a number"))?;
        if x == 0.0 {
            return Ok(py_tuple(vec![py_float(0.0), py_int(0)]));
        }
        let bits = x.to_bits();
        let biased_exp = ((bits >> 52) & 0x7ff) as i64;
        let normalized_exp = biased_exp - 1023;
        let mantissa_bits = bits & 0x000f_ffff_ffff_ffff;
        let sign_bit = bits & 0x8000_0000_0000_0000;
        // Reconstruct mantissa in range [0.5, 1)
        let fraction_bits = sign_bit | (0x3fe << 52) | mantissa_bits;
        let fraction = f64::from_bits(fraction_bits);
        Ok(py_tuple(vec![py_float(fraction), py_int(normalized_exp)]))
    });
    math_func!("ulp", |args| {
        if args.is_empty() { return Err(PyError::type_error("ulp() requires an argument")); }
        let x = math_arg_f64(&args[0]).ok_or_else(|| PyError::type_error("argument must be a number"))?;
        // Calculate ULP: distance to next representable float
        if x.is_nan() || x.is_infinite() { return Ok(py_float(x)); }
        if x == 0.0 { return Ok(py_float(f64::MIN_POSITIVE)); }
        let abs = x.abs();
        let next = if abs == f64::INFINITY { abs } else {
            let bits = abs.to_bits();
            f64::from_bits(bits + 1)
        };
        Ok(py_float(next - abs))
    });
    math_func!("nextafter", |args| {
        if args.len() < 2 { return Err(PyError::type_error("nextafter() requires 2 arguments")); }
        let x = math_arg_f64(&args[0]).ok_or_else(|| PyError::type_error("argument must be a number"))?;
        let y = math_arg_f64(&args[1]).ok_or_else(|| PyError::type_error("argument must be a number"))?;
        if x.is_nan() || y.is_nan() { return Ok(py_float(f64::NAN)); }
        if x == y { return Ok(py_float(x)); }
        if x == 0.0 {
            if y > 0.0 { return Ok(py_float(f64::MIN_POSITIVE)); }
            else { return Ok(py_float(-f64::MIN_POSITIVE)); }
        }
        let bits = x.to_bits();
        let next = if y > x { bits + 1 } else { bits - 1 };
        Ok(py_float(f64::from_bits(next)))
    });
    math_func!("prod", |args| {
        if args.is_empty() { return Err(PyError::type_error("prod() requires an argument")); }
        let start = if args.len() > 1 { args[1].as_i64().unwrap_or(1) } else { 1i64 };
        let obj = args[0].borrow();
        let mut result = num_bigint::BigInt::from(start);
        if let PyObject::List(items) = &*obj {
            for item in items {
                result *= num_bigint::BigInt::from(item.as_i64().unwrap_or(1));
            }
        } else if let PyObject::Tuple(items) = &*obj {
            for item in items {
                result *= num_bigint::BigInt::from(item.as_i64().unwrap_or(1));
            }
        }
        Ok(py_int(result))
    });

    // ── Constants ─────────────────────────────────────────────────────────
    d.insert_str("pi", py_float(std::f64::consts::PI));
    d.insert_str("e", py_float(std::f64::consts::E));
    d.insert_str("tau", py_float(std::f64::consts::TAU));
    d.insert_str("inf", py_float(f64::INFINITY));
    d.insert_str("nan", py_float(f64::NAN));
    d
}

// `sys.exc_info()` — a real top-level fn (not an inline closure in
// `create_sys_dict`) so `vm.rs`'s `call_function` can special-case it by
// `fn_addr_eq` pointer identity, running it through the real, live `&mut
// VirtualMachine` instead of `with_vm_mut`. `with_vm_mut`'s `VM_PTR` is set
// unconditionally before any bytecode executes, so calling it from HERE
// (itself only ever reached via a live bytecode CALL) always created a
// second, aliased `&mut VirtualMachine` on top of the one already active —
// real, unconditional UB, confirmed via the simplest possible repro
// (`try: raise ValueError("x") except Exception: sys.exc_info()`)
// reliably segfaulting. The exact same class of bug as the `exec()`/
// `eval()` fix earlier this session — `with_vm_mut` is fundamentally
// unsafe to call from any code path already running under a live VM
// (i.e. virtually always), not just some rare reentrant case.
pub fn sys_exc_info_builtin(_args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let result = crate::object::with_vm_mut(|vm| {
        py_tuple(vec![
            vm.exc_type.clone().unwrap_or(py_none()),
            vm.exc_value.clone().unwrap_or(py_none()),
            vm.exc_traceback.clone().unwrap_or(py_none()),
        ])
    });
    Ok(result.unwrap_or_else(|_| py_tuple(vec![py_none(), py_none(), py_none()])))
}

thread_local! {
    // `sys.settrace`/`gettrace`: stores the current global trace function.
    // This does NOT actually fire 'call'/'line'/'return'/'exception' trace
    // events during execution (that needs deep VM instrumentation — a much
    // larger feature, not attempted here) — it only makes the get/set
    // protocol itself work, which is enough for the extremely common
    // `self.addCleanup(sys.settrace, sys.gettrace())` /
    // `sys.settrace(None); assert sys.gettrace() is None` setup/teardown
    // pattern (real trigger: `test_sys_settrace.py`'s own test fixtures)
    // to stop raising `AttributeError: 'module' object has no attribute
    // 'settrace'` on every single test in the file, rather than crashing
    // before even reaching whatever the test itself checks.
    static CURRENT_TRACE_FUNC: std::cell::RefCell<Option<PyObjectRef>> = std::cell::RefCell::new(None);
}

pub fn sys_settrace_builtin(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let func = args.first().cloned().unwrap_or_else(py_none);
    CURRENT_TRACE_FUNC.with(|f| {
        *f.borrow_mut() = if matches!(&*func.borrow(), PyObject::None) { None } else { Some(func) };
    });
    Ok(py_none())
}

pub fn sys_gettrace_builtin(_args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    Ok(CURRENT_TRACE_FUNC.with(|f| f.borrow().clone()).unwrap_or_else(py_none))
}

pub fn sys_getrecursionlimit_builtin(_args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let result = crate::object::with_vm_mut(|vm| py_int(vm.recursion_limit as i64));
    Ok(result.unwrap_or_else(|_| py_int(1000)))
}

pub fn sys_setrecursionlimit_builtin(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let n = args.get(0).and_then(|a| a.as_i64()).ok_or_else(|| PyError::type_error("setrecursionlimit() requires an integer argument"))?;
    if n < 1 { return Err(PyError::value_error("recursion limit must be greater or equal than 1")); }
    let _ = crate::object::with_vm_mut(|vm| { vm.recursion_limit = n as usize; });
    Ok(py_none())
}

pub fn create_sys_dict(argv: Vec<String>) -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! sys_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }
    sys_func!("exit", |args| {
        let code = if args.len() > 0 {
            match &*args[0].borrow() {
                PyObject::Int(i) => i.to_i64().unwrap_or(0) as i32,
                _ => 1,
            }
        } else { 0 };
        Err(PyError::SystemExit(code))
    });
    sys_func!("displayhook", |args| {
        if args.is_empty() { return Ok(py_none()); }
        let val = &args[0];
        if matches!(&*val.borrow(), PyObject::None) {
            return Ok(py_none());
        }
        println!("{}", val.repr());
        Ok(py_none())
    });
    d.insert_str("argv", py_list(argv.into_iter().map(|s| py_str(&s)).collect()));
    d.insert_str("path", py_list(vec![]));
    d.insert_str("modules", py_dict());
    d.insert_str("warnoptions", py_list(vec![]));
    d.insert_str("version", py_str("3.12.0 (RustPython 0.1.0)"));
    d.insert_str("version_info", py_tuple(vec![py_int(3), py_int(12), py_int(0)]));
    d.insert_str("float_repr_style", py_str("short"));
    d.insert_str("hexversion", py_int(0x030c0000));
    // sys.flags — real CPython exposes this as a structseq (tuple +
    // attribute access). A plain Instance with these as attributes is
    // enough for real code that reads specific flags by name (e.g.
    // unittest's runner checking `sys.flags.dev_mode` transitively) — all
    // false/zero, matching "no special flags" for a script run normally.
    {
        let mut flags_dict = AttrMap::new();
        for flag in [
            "debug", "inspect", "interactive", "optimize", "dont_write_bytecode",
            "no_user_site", "no_site", "ignore_environment", "verbose",
            "bytes_warning", "quiet", "hash_randomization", "isolated",
            "dev_mode", "utf8_mode", "safe_path", "warn_default_encoding",
        ] {
            flags_dict.insert(flag.to_string(), py_int(0));
        }
        flags_dict.insert_str("int_max_str_digits", py_int(4300));
        d.insert_str("flags", PyObjectRef::new(PyObject::Instance {
            typ: PyObjectRef::new(PyObject::Type {
                name: "flags".to_string(),
                dict: Box::new(str_map_to_typedict(HashMap::new())),
                bases: vec![],
                mro: vec![],
            }),
            dict: flags_dict,
        }));
    }
    {
        // sys.hash_info — a real CPython structseq describing the hash
        // algorithm's parameters. Values match this interpreter's actual
        // hashing (a plain 64-bit `usize` computed directly, no SipHash
        // randomization) — `width`/`hash_bits` are the two fields real
        // code actually reads (`test.support`'s own `NHASHBITS = sys.
        // hash_info.width`); the rest are filled in for completeness/
        // structural parity with real CPython rather than because
        // anything here depends on them being exact.
        let mut hash_info_dict = AttrMap::new();
        hash_info_dict.insert_str("width", py_int(64));
        hash_info_dict.insert_str("modulus", py_int((1i64 << 61) - 1));
        hash_info_dict.insert_str("inf", py_int(314159));
        hash_info_dict.insert_str("nan", py_int(0));
        hash_info_dict.insert_str("imag", py_int(1000003));
        hash_info_dict.insert_str("algorithm", py_str("fnv"));
        hash_info_dict.insert_str("hash_bits", py_int(64));
        hash_info_dict.insert_str("seed_bits", py_int(128));
        hash_info_dict.insert_str("cutoff", py_int(0));
        d.insert_str("hash_info", PyObjectRef::new(PyObject::Instance {
            typ: PyObjectRef::new(PyObject::Type {
                name: "hash_info".to_string(),
                dict: Box::new(str_map_to_typedict(HashMap::new())),
                bases: vec![],
                mro: vec![],
            }),
            dict: hash_info_dict,
        }));
    }
    {
        // sys.int_info — describes the internal representation of Python
        // `int`. This interpreter backs arbitrary-precision ints with
        // `num-bigint`, not CPython's own 30-bit-digit array — these
        // values are CPython's OWN real constants (`bits_per_digit=30`,
        // `sizeof_digit=4`), reported for compatibility with code that
        // merely inspects them (e.g. `sys.int_info.bits_per_digit`)
        // without depending on this interpreter's actual internal
        // representation matching bit-for-bit.
        let mut int_info_dict = AttrMap::new();
        int_info_dict.insert_str("bits_per_digit", py_int(30));
        int_info_dict.insert_str("sizeof_digit", py_int(4));
        int_info_dict.insert_str("default_max_str_digits", py_int(4300));
        int_info_dict.insert_str("str_digits_check_threshold", py_int(640));
        d.insert_str("int_info", PyObjectRef::new(PyObject::Instance {
            typ: PyObjectRef::new(PyObject::Type {
                name: "int_info".to_string(),
                dict: Box::new(str_map_to_typedict(HashMap::new())),
                bases: vec![],
                mro: vec![],
            }),
            dict: int_info_dict,
        }));
    }
    {
        // sys.thread_info — describes the threading implementation. This
        // interpreter's own `threading` module is backed by real OS
        // threads (`std::thread`), which is exactly what CPython's own
        // "pthread" report describes on any POSIX platform.
        let mut thread_info_dict = AttrMap::new();
        thread_info_dict.insert_str("name", py_str("pthread"));
        thread_info_dict.insert_str("lock", py_str("mutex+cond"));
        thread_info_dict.insert_str("version", py_none());
        d.insert_str("thread_info", PyObjectRef::new(PyObject::Instance {
            typ: PyObjectRef::new(PyObject::Type {
                name: "thread_info".to_string(),
                dict: Box::new(str_map_to_typedict(HashMap::new())),
                bases: vec![],
                mro: vec![],
            }),
            dict: thread_info_dict,
        }));
    }
    {
        // sys.float_info — real CPython structseq describing the platform
        // `double` (matches Rust `f64`, IEEE 754 binary64 — same values
        // real CPython reports on any IEEE-754 platform, which is
        // effectively all of them).
        let mut float_info_dict = AttrMap::new();
        float_info_dict.insert_str("max", py_float(f64::MAX));
        float_info_dict.insert_str("max_exp", py_int(1024));
        float_info_dict.insert_str("max_10_exp", py_int(308));
        float_info_dict.insert_str("min", py_float(f64::MIN_POSITIVE));
        float_info_dict.insert_str("min_exp", py_int(-1021));
        float_info_dict.insert_str("min_10_exp", py_int(-307));
        float_info_dict.insert_str("dig", py_int(15));
        float_info_dict.insert_str("mant_dig", py_int(53));
        float_info_dict.insert_str("epsilon", py_float(f64::EPSILON));
        float_info_dict.insert_str("radix", py_int(2));
        float_info_dict.insert_str("rounds", py_int(1));
        d.insert_str("float_info", PyObjectRef::new(PyObject::Instance {
            typ: PyObjectRef::new(PyObject::Type {
                name: "float_info".to_string(),
                dict: Box::new(str_map_to_typedict(HashMap::new())),
                bases: vec![],
                mro: vec![],
            }),
            dict: float_info_dict,
        }));
    }
    {
        // sys._jit — CPython 3.13+'s experimental copy-and-patch JIT
        // introspection object (`sys._jit.is_enabled()`/`is_active()`).
        // Unrelated to this interpreter's own optional Cranelift `jit`
        // Cargo feature; either way the correct answer for test purposes
        // is "not enabled". Real trigger: `test.support`'s own
        // `_JIT_ENABLED = sys._jit.is_enabled()`.
        let mut jit_dict = AttrMap::new();
        jit_dict.insert_str("is_enabled", PyObjectRef::new(PyObject::BuiltinFunction {
            name: "is_enabled".to_string(),
            func: |_args| Ok(py_bool(false)),
        }));
        jit_dict.insert_str("is_active", PyObjectRef::new(PyObject::BuiltinFunction {
            name: "is_active".to_string(),
            func: |_args| Ok(py_bool(false)),
        }));
        d.insert_str("_jit", PyObjectRef::new(PyObject::Instance {
            typ: PyObjectRef::new(PyObject::Type {
                name: "_jit".to_string(),
                dict: Box::new(str_map_to_typedict(HashMap::new())),
                bases: vec![],
                mro: vec![],
            }),
            dict: jit_dict,
        }));
    }
    d.insert_str("stdin", PyObjectRef::new(PyObject::File {
        file: std::rc::Rc::new(std::cell::RefCell::new(dup_std_fd(0).unwrap_or_else(|_| {
            std::fs::File::open("/dev/null").unwrap()
        }))),
        name: "<stdin>".to_string(),
    }));
    d.insert_str("stdout", PyObjectRef::new(PyObject::File {
        file: std::rc::Rc::new(std::cell::RefCell::new(dup_std_fd(1).unwrap_or_else(|_| {
            std::fs::File::create("/dev/null").unwrap()
        }))),
        name: "<stdout>".to_string(),
    }));
    d.insert_str("stderr", PyObjectRef::new(PyObject::File {
        file: std::rc::Rc::new(std::cell::RefCell::new(dup_std_fd(2).unwrap_or_else(|_| {
            std::fs::File::create("/dev/null").unwrap()
        }))),
        name: "<stderr>".to_string(),
    }));
    d.insert_str("platform", py_str(std::env::consts::OS));
    // sys.implementation — CPython returns a namespace with name, cache_tag, etc.
    {
        let mut imp_dict = HashMap::new();
        imp_dict.insert_str("name", py_str("cpython"));
        imp_dict.insert_str("cache_tag", py_str("cpython-314"));
        imp_dict.insert_str("hexversion", py_int(50987248)); // 0x030A0000
        imp_dict.insert_str("_multi_threaded", py_bool(true));
        d.insert_str("implementation", create_module("implementation", imp_dict));
    }
    d.insert_str("byteorder", py_str(if cfg!(target_endian = "little") { "little" } else { "big" }));
    d.insert_str("maxsize", py_int(i64::MAX));
    d.insert_str("maxunicode", py_int(1114111));
    d.insert_str("api_version", py_int(1013));
    d.insert_str("executable", py_str(&std::env::current_exe().map(|p| p.to_string_lossy().to_string()).unwrap_or_default()));
    // Detect virtual environment (uv, venv, virtualenv, conda, poetry, pixi)
    let venv_path = std::env::var("VIRTUAL_ENV").ok()
        .or_else(|| std::env::var("CONDA_PREFIX").ok())
        .or_else(|| {
            // Poetry: POETRY_ACTIVE is set when inside a poetry shell
            // Also check POETRY_VIRTUAL_ENV which poetry sets explicitly
            if std::env::var("POETRY_ACTIVE").is_ok() {
                std::env::var("POETRY_VIRTUAL_ENV").ok()
            } else {
                None
            }
        })
        .or_else(|| {
            // Pixi environments
            std::env::var("PIXI_IN_SHELL").ok().and_then(|_| std::env::var("PIXI_PROJECT_ROOT").ok())
        })
        .or_else(|| {
            // Also look for .venv in CWD
            let cwd = std::env::current_dir().ok()?;
            let dot_venv = cwd.join(".venv");
            if dot_venv.is_dir() { Some(dot_venv.to_string_lossy().to_string()) } else { None }
        });
    let (prefix, exec_prefix) = if let Some(ref venv) = venv_path {
        (venv.clone(), venv.clone())
    } else {
        ("/usr".to_string(), "/usr".to_string())
    };
    d.insert_str("prefix", py_str(&prefix));
    d.insert_str("exec_prefix", py_str(&exec_prefix));
    // `sys.base_prefix`/`base_exec_prefix` — real CPython's own venv-
    // detection idiom (`sys.prefix != sys.base_prefix`) needs these to be
    // the REAL, non-venv installation prefix regardless of whether a venv
    // is currently active — unlike `prefix`/`exec_prefix` above (which
    // deliberately follow the active venv). Was missing entirely, meaning
    // any code doing this exact "am I in a venv" check (a common pattern
    // — `pip`, `venv` itself, build tooling) raised `AttributeError`
    // instead of getting a straight answer.
    d.insert_str("base_prefix", py_str("/usr"));
    d.insert_str("base_exec_prefix", py_str("/usr"));
    d.insert_str("winver", py_str("3.12"));
    // sys.exc_info() — returns current exception info from VM. Real logic
    // lives in `sys_exc_info_builtin` (a real top-level fn, not inlined
    // here) so `vm.rs`'s `call_function` can recognize and special-case it
    // by pointer identity — see the fix there for why `with_vm_mut` alone
    // is unsafe.
    sys_func!("getfilesystemencoding", |_args| Ok(py_str("utf-8")));
    sys_func!("getfilesystemencodeerrors", |_args| Ok(py_str("surrogateescape")));
    sys_func!("getdefaultencoding", |_args| Ok(py_str("utf-8")));
    sys_func!("exc_info", sys_exc_info_builtin);
    sys_func!("getrecursionlimit", sys_getrecursionlimit_builtin);
    sys_func!("setrecursionlimit", sys_setrecursionlimit_builtin);
    sys_func!("settrace", sys_settrace_builtin);
    sys_func!("gettrace", sys_gettrace_builtin);
    sys_func!("_getframe", |args| {
        let level = if args.is_empty() { 0 } else { args[0].as_i64().unwrap_or(0) };
        // Return a basic frame representation
        Ok(py_none())
    });
    sys_func!("get_int_max_str_digits", |_| {
        Ok(py_int(crate::object::INT_MAX_STR_DIGITS.with(|d| d.get())))
    });
    sys_func!("set_int_max_str_digits", |args| {
        let val = if args.len() >= 1 { args[0].as_i64().unwrap_or(4300) } else { 4300 };
        crate::object::INT_MAX_STR_DIGITS.with(|d| d.set(val));
        Ok(py_none())
    });
    // `sys.getsizeof(obj)` — was missing entirely. Real CPython reports the
    // actual C-level memory footprint of `obj`, which has no equivalent
    // meaning against this interpreter's own, completely different object
    // representation — so this is a deliberate APPROXIMATION (rough,
    // per-type base size + a per-element estimate for containers) good
    // enough for code that just checks `getsizeof(x) > 0` or compares
    // relative sizes between two objects of the same type, not for code
    // asserting on an exact byte count (which would be fragile even
    // between two real CPython builds/versions anyway).
    sys_func!("getsizeof", |args| {
        if args.is_empty() { return Err(PyError::type_error("getsizeof() takes at least 1 argument")); }
        let size = match &*args[0].borrow() {
            PyObject::None => 16,
            PyObject::Bool(_) => 28,
            PyObject::Int(_) => 28,
            PyObject::Float(_) => 24,
            PyObject::Str(s) => 49 + s.len() as i64,
            PyObject::Bytes(b) => 33 + b.len() as i64,
            PyObject::ByteArray(b) => 56 + b.len() as i64,
            PyObject::List(v) => 56 + 8 * v.len() as i64,
            PyObject::Tuple(v) => 40 + 8 * v.len() as i64,
            PyObject::Dict(d) => 64 + 32 * d.len() as i64,
            PyObject::Set(s) | PyObject::FrozenSet(s) => 216 + 32 * s.len() as i64,
            _ => 48,
        };
        Ok(py_int(size))
    });
    d
}

    /// Helper: resolve a module name with relative import support
    fn resolve_name(name: &str, package: Option<&str>) -> Result<String, PyError> {
        if !name.starts_with('.') {
            return Ok(name.to_string());
        }
        let pkg = match package {
            Some(p) => p.to_string(),
            None => return Err(PyError::type_error(
                "import_module() requires 'package' argument for relative import"
            )),
        };
        let level = name.chars().take_while(|&c| c == '.').count();
        let rel_part = &name[level..];
        let pkg_parts: Vec<&str> = pkg.split('.').collect();
        if level > pkg_parts.len() {
            return Err(PyError::ImportError(
                "attempted relative import beyond top-level package".to_string()
            ));
        }
        let base = &pkg_parts[..pkg_parts.len() - level];
        if base.is_empty() {
            Ok(rel_part.to_string())
        } else if rel_part.is_empty() {
            Ok(base.join("."))
        } else {
            Ok(format!("{}.{}", base.join("."), rel_part))
        }
    }

    /// Helper: import a dotted module chain, ensuring parents are loaded first
    fn import_dotted(vm: &mut crate::vm::VirtualMachine, name: &str) -> PyResult<PyObjectRef> {
        // If it's already loaded, return it
        if let Some(module) = vm.modules.get(name) {
            return Ok(module.clone());
        }
        // For dotted names, load the chain step by step
        if name.contains('.') {
            let parts: Vec<&str> = name.split('.').collect();
            let mut current = String::new();
            for part in &parts {
                if current.is_empty() {
                    current = part.to_string();
                } else {
                    current = format!("{}.{}", current, part);
                }
                if !vm.modules.contains_key(&current) {
                    let module = vm.import_module_from_file(&current)?;
                    vm.modules.insert(current.clone(), module.clone());
                    // Also sync to sys.modules
                    if let Some(sys_mod) = vm.modules.get("sys") {
                        if let PyObject::Module { dict, .. } = &*sys_mod.borrow() {
                            if let Some(mod_dict) = dict.get_str("modules") {
                                mod_dict.borrow_mut().set_attribute(&current, module).ok();
                            }
                        }
                    }
                }
            }
            if let Some(module) = vm.modules.get(name) {
                return Ok(module.clone());
            }
            return Err(PyError::ImportError(format!("No module named '{}'", name)));
        }
        // Simple name
        let module = vm.import_module_from_file(name)?;
        vm.modules.insert(name.to_string(), module.clone());
        if let Some(sys_mod) = vm.modules.get("sys") {
            if let PyObject::Module { dict, .. } = &*sys_mod.borrow() {
                if let Some(mod_dict) = dict.get_str("modules") {
                    mod_dict.borrow_mut().set_attribute(name, module.clone()).ok();
                }
            }
        }
        Ok(module)
    }

    /// `importlib.import_module(name, package=None)`. A genuine, named,
    /// top-level function (not an inline closure like this module's other
    /// builtins) specifically so `vm.rs`'s `call_function` can recognize it
    /// by function-pointer identity and special-case it — matching
    /// `type.__new__`/`getattr` above. `with_vm_mut` below is only a
    /// fallback for the (currently believed unreachable, since every real
    /// call goes through a normal `CALL`/`CALL_KW` opcode) case of being
    /// invoked some other way; the aliasing hazard it otherwise risks
    /// (see `with_vm_mut`'s own doc comment) is why the special case exists.
    pub(crate) fn import_module_builtin(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
        if args.is_empty() {
            return Err(PyError::type_error("import_module() missing required argument 'name'"));
        }
        let name = args[0].str();
        let package: Option<String> = if args.len() >= 2 {
            let pkg = args[1].str();
            if pkg.is_empty() { None } else { Some(pkg) }
        } else { None };

        // Resolve relative imports
        let resolved = resolve_name(&name, package.as_deref())?;

        // Use with_vm_mut for VM-dependent part
        with_vm_mut(|vm| -> PyResult<PyObjectRef> {
            if let Some(module) = vm.modules.get(&resolved) {
                return Ok(module.clone());
            }
            import_dotted(vm, &resolved)
        })?
    }

    /// Shared by both `call_function`'s special case (the normal path) and
    /// the plain-`BuiltinFunction` fallback: resolves relative imports and
    /// returns the already-loaded module or imports it fresh via `vm`.
    pub(crate) fn import_module_with_vm(vm: &mut crate::vm::VirtualMachine, name: &str, package: Option<&str>) -> PyResult<PyObjectRef> {
        let resolved = resolve_name(name, package)?;
        if let Some(module) = vm.modules.get(&resolved) {
            return Ok(module.clone());
        }
        import_dotted(vm, &resolved)
    }

/// Native importlib stub module providing import_module().
pub fn create_importlib_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    d.insert_str("import_module", PyObjectRef::new(PyObject::BuiltinFunction { name: "import_module".to_string(), func: import_module_builtin }));
    // __version__ — indicates importlib metadata
    d.insert_str("__version__", py_str("1.0.0"));
    // `importlib.invalidate_caches()` — real CPython clears internal
    // finder/loader caches so newly-created files on disk (a common test
    // pattern: write a module file, then import it) are found. This
    // interpreter's own import machinery doesn't maintain any such cache to
    // begin with (every import does a fresh filesystem lookup), so a no-op
    // is the correct, safe simplification — missing entirely before raised
    // `AttributeError`, breaking any test that merely CALLS this for
    // hygiene even when it doesn't strictly need caches invalidated (real
    // trigger: CPython's own `test_cmd_line_script.py`/`test_tokenize.py`/
    // others).
    d.insert_str("invalidate_caches", PyObjectRef::new(PyObject::BuiltinFunction {
        name: "invalidate_caches".to_string(),
        func: |_args| Ok(py_none()),
    }));
    d
}

/// Native importlib.util module providing find_spec().
pub fn create_importlib_util_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! util_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    // find_spec(name, package=None) -> ModuleSpec or None
    util_func!("find_spec", find_spec_builtin);

    // cache_from_source(path, ...)/source_from_cache(path) — real CPython's
    // `__pycache__/name.cpython-VER.pyc` naming convention. Implemented as
    // plain string manipulation (not tied to this interpreter's own actual
    // bytecode-cache format) — good enough for code that just constructs/
    // parses the conventional path shape (real trigger: `py_compile.py`,
    // vendored verbatim, needs `cache_from_source` to pick a default output
    // path for `py_compile.compile()`).
    util_func!("cache_from_source", |args| {
        if args.is_empty() { return Err(PyError::type_error("cache_from_source() missing required argument: 'path'")); }
        let path = args[0].str();
        let (dir, base) = match path.rfind('/') {
            Some(i) => (path[..i].to_string(), path[i+1..].to_string()),
            None => (String::new(), path.clone()),
        };
        let stem = base.strip_suffix(".py").unwrap_or(&base);
        let cache_dir = if dir.is_empty() { "__pycache__".to_string() } else { format!("{}/__pycache__", dir) };
        Ok(py_str(&format!("{}/{}.cpython-314.pyc", cache_dir, stem)))
    });
    util_func!("source_from_cache", |args| {
        if args.is_empty() { return Err(PyError::type_error("source_from_cache() missing required argument: 'path'")); }
        let path = args[0].str();
        if !path.ends_with(".pyc") {
            return Err(PyError::value_error("not a valid pyc path"));
        }
        let without_pycache = path.replace("/__pycache__/", "/");
        let base = without_pycache.rsplit('/').next().unwrap_or(&without_pycache);
        let dir = without_pycache[..without_pycache.len() - base.len()].to_string();
        let stem = base.split(".cpython-").next().unwrap_or(base);
        Ok(py_str(&format!("{}{}.py", dir, stem)))
    });

    d
}

/// The real body of `importlib.util.find_spec`, given genuine `&mut
/// VirtualMachine` access — called directly from `vm.rs`'s `call_function`
/// special-case (see the `is_find_spec` check there) instead of through
/// `find_spec_builtin`'s `with_vm_mut` fallback below, since this function is
/// always reached from deep inside an active VM call chain in practice
/// (Django's app-loading machinery calls it while `apps.populate()` is
/// running), and `with_vm_mut` there reborrows the *same* `VirtualMachine`
/// mutably while an outer `&mut self` is already live on the Rust call stack
/// — a real, confirmed aliasing UB (`hashbrown`'s `HashMap::contains_key`
/// segfaulting on a corrupted table, non-deterministically, since the bug is
/// UB and not always "caught"), not merely a style concern.
pub(crate) fn find_spec_with_vm(vm: &mut crate::vm::VirtualMachine, name: &str, package: Option<&str>) -> PyResult<PyObjectRef> {
    // Resolve the full module name (handle relative imports)
    let resolved_name = if let Some(pkg) = package {
        if name.starts_with('.') {
            let level = name.chars().take_while(|&c| c == '.').count();
            let rel_part = &name[level..];
            let pkg_parts: Vec<&str> = pkg.split('.').collect();
            if level > pkg_parts.len() {
                return Ok(py_none());
            }
            let base = &pkg_parts[..pkg_parts.len() - level];
            if base.is_empty() {
                rel_part.to_string()
            } else if rel_part.is_empty() {
                base.join(".")
            } else {
                format!("{}.{}", base.join("."), rel_part)
            }
        } else if !name.contains('.') {
            format!("{}.{}", pkg, name)
        } else {
            name.to_string()
        }
    } else {
        name.to_string()
    };

    if vm.modules.contains_key(&resolved_name) {
        return Ok(create_module("ModuleSpec", HashMap::from([
            ("name".to_string(), py_str(&resolved_name)),
            ("origin".to_string(), py_str("built-in")),
        ])));
    }

    // Get sys.path manually to search for the module file
    let mut search_paths: Vec<String> = Vec::new();
    if let Some(sys_mod) = vm.modules.get("sys") {
        if let PyObject::Module { dict, .. } = &*sys_mod.borrow() {
            if let Some(path_list) = dict.get_str("path") {
                if let PyObject::List(items) = &*path_list.borrow() {
                    for item in items {
                        if let PyObject::Str(s) = &*item.borrow() {
                            search_paths.push(s.to_string());
                        }
                    }
                }
            }
        }
    }

    // For dotted names, we need to find the file for the top-level
    let top_name = if resolved_name.contains('.') {
        resolved_name.split('.').next().unwrap().to_string()
    } else {
        resolved_name.clone()
    };

    // Search the filesystem for the module
    for base in &search_paths {
        let base_trimmed = base.trim_end_matches('/');
        let py_path = format!("{}/{}.py", base_trimmed, top_name);
        if std::path::Path::new(&py_path).exists() {
            return Ok(create_module("ModuleSpec", HashMap::from([
                ("name".to_string(), py_str(&resolved_name)),
                ("origin".to_string(), py_str(&py_path)),
            ])));
        }
        let init_path = format!("{}/{}/__init__.py", base_trimmed, top_name);
        if std::path::Path::new(&init_path).exists() {
            return Ok(create_module("ModuleSpec", HashMap::from([
                ("name".to_string(), py_str(&resolved_name)),
                ("origin".to_string(), py_str(&init_path)),
            ])));
        }
    }

    Ok(py_none())
}

/// `find_spec`'s standalone entry point (used when it's not reached through
/// `vm.rs`'s special-cased dispatch) — falls back to `with_vm_mut`, matching
/// `import_module_builtin`'s role for `importlib.import_module`.
pub(crate) fn find_spec_builtin(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error("find_spec() missing required argument 'name'"));
    }
    let name = args[0].str();
    let package = if args.len() >= 2 {
        let pkg = args[1].str();
        if pkg.is_empty() { None } else { Some(pkg) }
    } else { None };
    Ok(with_vm_mut(|vm| find_spec_with_vm(vm, &name, package.as_deref()))??)
}

/// Native importlib.resources stub module.
/// Provides `files(package)` and `as_file(traversable)` stubs for certifi compatibility.
pub fn create_importlib_resources_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();

    // Helper: read name attribute from a module
    fn mod_name(obj: &PyObjectRef) -> String {
        let b = obj.borrow();
        if let PyObject::Module { dict, .. } = &*b {
            if let Some(name) = dict.get_str("name") {
                if let PyObject::Str(s) = &*name.borrow() {
                    return s.to_string();
                }
            }
        }
        String::new()
    }

    // __enter__ for context manager: return args[0].name
    fn enter_cm(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
        if args.is_empty() { return Ok(py_none()); }
        Ok(py_str(&mod_name(&args[0])))
    }

    // __exit__ for context manager: no-op
    fn exit_cm(_args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
        Ok(py_none())
    }

    // joinpath for traversable: args[0].name + args[1], returns new Traversable
    fn trav_joinpath(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
        if args.len() < 2 { return Ok(py_none()); }
        let base = mod_name(&args[0]);
        let child = args[1].str();
        let joined = format!("{}/{}", base.trim_end_matches('/'), child.trim_start_matches('/'));
        let trav = create_module("_Traversable", HashMap::from([
            ("name".to_string(), py_str(&joined)),
        ]));
        // Add joinpath as BuiltinMethod with self_obj = trav
        if let PyObject::Module { dict, .. } = &mut *trav.borrow_mut() {
            dict.insert_str("joinpath", PyObjectRef::new(PyObject::BuiltinMethod {
                name: "joinpath".to_string(),
                func: trav_joinpath,
                self_obj: trav.clone(),
            }));
        }
        Ok(trav)
    }

    // as_file(traversable) -> context manager wrapping the path
    d.insert_str("as_file", PyObjectRef::new(PyObject::BuiltinFunction {
        name: "as_file".to_string(),
        func: |args| {
            if args.is_empty() {
                return Err(PyError::type_error("as_file() requires 1 argument (traversable)"));
            }
            let path_str = mod_name(&args[0]);
            if path_str.is_empty() {
                return Err(PyError::type_error("as_file() requires traversable with a valid name"));
            }
            let cm = create_module("_CtxManager", HashMap::from([
                ("name".to_string(), py_str(&path_str)),
            ]));
            // Add __enter__/__exit__ as BuiltinMethod with self_obj = cm
            // so that when called via module.__enter__(), the function receives
            // the module itself as args[0] (via BuiltinMethod self-binding).
            if let PyObject::Module { dict, .. } = &mut *cm.borrow_mut() {
                dict.insert_str("__enter__", PyObjectRef::new(PyObject::BuiltinMethod {
                    name: "__enter__".to_string(),
                    func: enter_cm,
                    self_obj: cm.clone(),
                }));
                dict.insert_str("__exit__", PyObjectRef::new(PyObject::BuiltinMethod {
                    name: "__exit__".to_string(),
                    func: exit_cm,
                    self_obj: cm.clone(),
                }));
            }
            Ok(cm)
        },
    }));

    // files(package) -> traversable with joinpath()
    d.insert_str("files", PyObjectRef::new(PyObject::BuiltinFunction {
        name: "files".to_string(),
        func: |args| {
            if args.is_empty() {
                return Err(PyError::type_error("files() requires 1 argument (package name)"));
            }
            let pkg_name = args[0].str();
            // Look up the package's __path__ via VM_PTR
            let pkg_path: String = with_vm_mut(|vm| -> PyResult<String> {
                match vm.modules.get(&pkg_name) {
                    Some(mod_obj) => {
                        let borrowed = mod_obj.borrow();
                        if let PyObject::Module { dict, .. } = &*borrowed {
                            if let Some(path_list) = dict.get_str("__path__") {
                                if let PyObject::List(items) = &*path_list.borrow() {
                                    if let Some(first) = items.first() {
                                        if let PyObject::Str(s) = &*first.borrow() {
                                            Ok(s.to_string())
                                        } else { Ok(format!("./{}", pkg_name)) }
                                    } else { Ok(format!("./{}", pkg_name)) }
                                } else { Ok(format!("./{}", pkg_name)) }
                            } else { Ok(format!("./{}", pkg_name)) }
                        } else { Ok(format!("./{}", pkg_name)) }
                    }
                    None => Ok(format!("./{}", pkg_name)),
                }
            })??;

            let trav = create_module("_Traversable", HashMap::from([
                ("name".to_string(), py_str(&pkg_path)),
            ]));
            // Add joinpath as BuiltinMethod with self_obj = trav
            // so that when called via traversable.joinpath(...), the function receives
            // the traversable itself as args[0] (via BuiltinMethod self-binding).
            if let PyObject::Module { dict, .. } = &mut *trav.borrow_mut() {
                dict.insert_str("joinpath", PyObjectRef::new(PyObject::BuiltinMethod {
                    name: "joinpath".to_string(),
                    func: trav_joinpath,
                    self_obj: trav.clone(),
                }));
            }
            // __str__ via name attribute
            Ok(trav)
        },
    }));

    d
}

pub fn create_os_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! os_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }
    d.insert_str("curdir", py_str("."));
    d.insert_str("pardir", py_str(".."));
    d.insert_str("sep", py_str(if cfg!(windows) { "\\" } else { "/" }));
    d.insert_str("altsep", if cfg!(windows) { py_str("/") } else { py_none() });
    d.insert_str("extsep", py_str("."));
    d.insert_str("pathsep", py_str(if cfg!(windows) { ";" } else { ":" }));
    d.insert_str("linesep", py_str(if cfg!(windows) { "\r\n" } else { "\n" }));
    d.insert_str("defpath", py_str(if cfg!(windows) { "." } else { ":/bin:/usr/bin" }));
    d.insert_str("devnull", py_str(if cfg!(windows) { "nul" } else { "/dev/null" }));
    os_func!("fspath", |args| {
        if args.is_empty() { return Err(PyError::type_error("fspath() missing required argument: 'path'")); }
        let obj = args[0].clone();
        match &*obj.borrow() {
            PyObject::Str(_) | PyObject::Bytes(_) => return Ok(obj.clone()),
            _ => {}
        }
        if let Ok(f) = obj.borrow().get_attribute("__fspath__") {
            return call_bound_method(f, obj.clone(), vec![]);
        }
        Err(PyError::type_error(format!("expected str, bytes or os.PathLike object, not {}", obj.borrow().type_name())))
    });
    os_func!("fsencode", |args| {
        if args.is_empty() { return Err(PyError::type_error("fsencode() missing required argument: 'filename'")); }
        let s = args[0].str();
        Ok(PyObjectRef::imm(PyObject::Bytes(s.into_bytes())))
    });
    os_func!("fsdecode", |args| {
        if args.is_empty() { return Err(PyError::type_error("fsdecode() missing required argument: 'filename'")); }
        let obj = args[0].borrow();
        match &*obj {
            PyObject::Bytes(b) => Ok(py_str(&String::from_utf8_lossy(b))),
            PyObject::Str(s) => Ok(py_str(s)),
            _ => Err(PyError::type_error("expected str or bytes")),
        }
    });
    os_func!("listdir", |args| {
        let path = if args.len() > 0 { args[0].str() } else { ".".to_string() };
        match std::fs::read_dir(&path) {
            Ok(entries) => {
                let names: Vec<PyObjectRef> = entries.filter_map(|e| e.ok()).map(|e| py_str(&e.file_name().to_string_lossy())).collect();
                Ok(py_list(names))
            }
            Err(e) => Err(PyError::OsError(format!("{}", e))),
        }
    });
    os_func!("mkdir", |args| {
        if args.is_empty() { return Err(PyError::type_error("mkdir() takes at least 1 argument")); }
        match std::fs::create_dir(&args[0].str()) {
            Ok(()) => Ok(py_none()),
            Err(e) => Err(PyError::OsError(format!("{}", e))),
        }
    });
    os_func!("remove", |args| {
        if args.is_empty() { return Err(PyError::type_error("remove() takes at least 1 argument")); }
        let path = crate::object::path_arg_to_string(&args[0]);
        std::fs::remove_file(&path).map_err(|e| PyError::OsError(format!("{}", e)))?;
        Ok(py_none())
    });

    // os.unlink = os.remove (POSIX alias)
    os_func!("unlink", |args| {
        if args.is_empty() { return Err(PyError::type_error("unlink() takes at least 1 argument")); }
        let path = crate::object::path_arg_to_string(&args[0]);
        std::fs::remove_file(&path).map_err(|e| PyError::OsError(format!("{}", e)))?;
        Ok(py_none())
    });

    os_func!("rename", |args| {
        if args.len() < 2 { return Err(PyError::type_error("rename() takes 2 arguments")); }
        match std::fs::rename(&crate::object::path_arg_to_string(&args[0]), &crate::object::path_arg_to_string(&args[1])) {
            Ok(()) => Ok(py_none()),
            Err(e) => Err(PyError::OsError(format!("{}", e))),
        }
    });

    os_func!("system", |args| {
        if args.is_empty() { return Err(PyError::type_error("system() takes at least 1 argument")); }
        let cmd = args[0].str();
        match std::process::Command::new("/bin/sh").arg("-c").arg(&cmd).status() {
            Ok(status) => Ok(py_int(status.code().unwrap_or(0) as i64)),
            Err(e) => Err(PyError::OsError(format!("{}", e))),
        }
    });

    os_func!("chdir", |args| {
        if args.is_empty() { return Err(PyError::type_error("chdir() takes at least 1 argument")); }
        match std::env::set_current_dir(&args[0].str()) {
            Ok(()) => Ok(py_none()),
            Err(e) => Err(PyError::OsError(format!("{}", e))),
        }
    });

    os_func!("getcwd", |_| {
        match std::env::current_dir() {
            Ok(p) => Ok(py_str(&p.to_string_lossy())),
            Err(e) => Err(PyError::OsError(format!("{}", e))),
        }
    });

    os_func!("getenv", |args| {
        if args.is_empty() { return Ok(py_none()); }
        let key = args[0].str();
        match std::env::var(&key) {
            Ok(val) => Ok(py_str(&val)),
            Err(_) => {
                if args.len() > 1 { Ok(args[1].clone()) }
                else { Ok(py_none()) }
            }
        }
    });

    os_func!("putenv", |args| {
        if args.len() < 2 { return Err(PyError::type_error("putenv() takes exactly 2 arguments")); }
        std::env::set_var(args[0].str(), args[1].str());
        Ok(py_none())
    });

    os_func!("unsetenv", |args| {
        if args.is_empty() { return Err(PyError::type_error("unsetenv() takes at least 1 argument")); }
        std::env::remove_var(args[0].str());
        Ok(py_none())
    });

    // File descriptor operations
    os_func!("open", |args| {
        if args.len() < 2 { return Err(PyError::type_error("open() requires at least 2 arguments")); }
        let path = args[0].str();
        let flags = args[1].as_i64().unwrap_or(0) as i32;
        let mut opts = std::fs::OpenOptions::new();
        // O_RDONLY=0, O_WRONLY=1, O_RDWR=2 — check access mode
        let access_mode = flags & 3;
        if access_mode == 0 { opts.read(true); }     // O_RDONLY
        if access_mode == 1 { opts.write(true); }    // O_WRONLY
        if access_mode == 2 { opts.write(true); opts.read(true); } // O_RDWR
        if flags & 64 != 0 {       // O_CREAT = 64
            if flags & 128 != 0 {  // O_EXCL = 128
                opts.create_new(true);
            } else {
                opts.create(true);
            }
        }
        if flags & 512 != 0 { opts.truncate(true); }    // O_TRUNC = 512
        if flags & 1024 != 0 { opts.append(true); }     // O_APPEND = 1024
        match opts.open(&path) {
            Ok(file) => {
                use std::os::unix::io::IntoRawFd;
                Ok(py_int(file.into_raw_fd() as i64))
            }
            Err(e) => Err(PyError::OsError(format!("{}", e))),
        }
    });
    os_func!("read", |args| {
        if args.len() < 2 { return Err(PyError::type_error("read() requires at least 2 arguments")); }
        let fd = args[0].as_i64().unwrap_or(-1) as i32;
        let n = args[1].as_i64().unwrap_or(0) as usize;
        let mut buf = vec![0u8; n];
        match read_fd(fd, &mut buf) {
            Ok(count) => {
                buf.truncate(count);
                Ok(PyObjectRef::new(PyObject::Bytes(buf)))
            }
            Err(e) => {
                Err(PyError::OsError(format!("{}", e)))
            }
        }
    });
    os_func!("write", |args| {
        if args.len() < 2 { return Err(PyError::type_error("write() requires at least 2 arguments")); }
        let fd = args[0].as_i64().unwrap_or(-1) as i32;
        let data = match &*args[1].borrow() {
            PyObject::Bytes(b) => b.clone(),
            PyObject::Str(s) => s.as_bytes().to_vec(),
            _ => return Err(PyError::type_error("write() argument 2 must be bytes or str")),
        };
        match write_fd(fd, &data) {
            Ok(count) => {
                Ok(py_int(count as i64))
            }
            Err(e) => {
                Err(PyError::OsError(format!("{}", e)))
            }
        }
    });
    os_func!("close", |args| {
        if args.is_empty() { return Err(PyError::type_error("close() requires at least 1 argument")); }
        let fd = args[0].as_i64().unwrap_or(-1) as i32;
        close_fd(fd);
        Ok(py_none())
    });

    // os.fdopen(fd, mode='r') -> file object from fd
    os_func!("fdopen", |args| {
        if args.is_empty() { return Err(PyError::type_error("fdopen() missing required argument 'fd'")); }
        let fd = args[0].as_i64().ok_or_else(|| PyError::type_error("fd must be an integer"))? as i32;
        let _mode = if args.len() > 1 { args[1].str() } else { "r".to_string() };
        use std::os::unix::io::FromRawFd;
        let file = unsafe { std::fs::File::from_raw_fd(fd) };
        Ok(PyObjectRef::new(PyObject::File {
            file: std::rc::Rc::new(std::cell::RefCell::new(file)),
            name: format!("<fdopen>"),
        }))
    });

    // os.urandom(size) -> random bytes from OS
    os_func!("urandom", |args| {
        if args.is_empty() { return Err(PyError::type_error("urandom() requires at least 1 argument")); }
        let n = args[0].as_i64().ok_or_else(|| PyError::type_error("argument must be an integer"))?;
        if n <= 0 {
            return Ok(PyObjectRef::imm(PyObject::Bytes(Vec::new())));
        }
        let mut buf = vec![0u8; n as usize];
        if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
            use std::io::Read;
            let _ = f.read_exact(&mut buf);
        }
        Ok(PyObjectRef::imm(PyObject::Bytes(buf)))
    });

    // OS flags for open()
    d.insert_str("O_RDONLY", py_int(0));
    d.insert_str("O_WRONLY", py_int(1));
    d.insert_str("O_RDWR", py_int(2));
    d.insert_str("O_CREAT", py_int(64));
    d.insert_str("O_EXCL", py_int(128));
    d.insert_str("O_TRUNC", py_int(512));
    d.insert_str("O_APPEND", py_int(1024));

    // environ dict — use a proper PyDict instead of Module so methods like
    // .setdefault(), .get(), .keys(), 'x in environ', etc. all work (Django req.)
    let mut environ_pydict = PyDict::new();
    for (key, val) in std::env::vars() {
        environ_pydict.set(py_str(&key), py_str(&val)).ok();
    }
    d.insert_str("environ", PyObjectRef::new(PyObject::Dict(Box::new(environ_pydict))));

    // --- os.getpid() ---
    os_func!("getpid", |_| {
        Ok(py_int(std::process::id() as i64))
    });

    // --- os.getppid() ---
    os_func!("getppid", |_| {
        // Parse /proc/self/stat for parent PID
        match std::fs::read_to_string("/proc/self/stat") {
            Ok(stat) => {
                if let Some(idx) = stat.rfind(')') {
                    let fields: Vec<&str> = stat[idx+1..].split_whitespace().collect();
                    if fields.len() > 1 {
                        if let Ok(ppid) = fields[1].parse::<i64>() {
                            return Ok(py_int(ppid));
                        }
                    }
                }
                Err(PyError::OsError("failed to parse /proc/self/stat".to_string()))
            }
            Err(e) => Err(PyError::OsError(format!("{}", e))),
        }
    });

    // --- os.cpu_count() ---
    os_func!("cpu_count", |_| {
        match std::thread::available_parallelism() {
            Ok(n) => Ok(py_int(n.get() as i64)),
            Err(_) => Ok(py_none()),
        }
    });

    // --- os.getloadavg() ---
    os_func!("getloadavg", |_| {
        match std::fs::read_to_string("/proc/loadavg") {
            Ok(data) => {
                let parts: Vec<&str> = data.split_whitespace().collect();
                if parts.len() >= 3 {
                    let load1: f64 = parts[0].parse().unwrap_or(0.0);
                    let load5: f64 = parts[1].parse().unwrap_or(0.0);
                    let load15: f64 = parts[2].parse().unwrap_or(0.0);
                    Ok(py_tuple(vec![py_float(load1), py_float(load5), py_float(load15)]))
                } else {
                    Ok(py_tuple(vec![py_float(0.0), py_float(0.0), py_float(0.0)]))
                }
            }
            Err(_) => Ok(py_tuple(vec![py_float(0.0), py_float(0.0), py_float(0.0)])),
        }
    });

    // --- Helper: convert fs::Metadata to stat dict ---
    fn stat_to_dict(meta: &std::fs::Metadata) -> HashMap<String, PyObjectRef> {
        use std::os::unix::fs::MetadataExt;
        let mut d = HashMap::new();
        d.insert_str("st_mode", py_int(meta.mode() as i64));
        d.insert_str("st_ino", py_int(meta.ino() as i64));
        d.insert_str("st_dev", py_int(meta.dev() as i64));
        d.insert_str("st_nlink", py_int(meta.nlink() as i64));
        d.insert_str("st_uid", py_int(meta.uid() as i64));
        d.insert_str("st_gid", py_int(meta.gid() as i64));
        d.insert_str("st_size", py_int(meta.size() as i64));
        if let Ok(t) = meta.modified() {
            let dur = t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
            d.insert_str("st_mtime", py_float(dur.as_secs_f64()));
        }
        if let Ok(t) = meta.accessed() {
            let dur = t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
            d.insert_str("st_atime", py_float(dur.as_secs_f64()));
        }
        if let Ok(t) = meta.created() {
            let dur = t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
            d.insert_str("st_ctime", py_float(dur.as_secs_f64()));
        }
        d
    }

    // --- os.stat(path) ---
    os_func!("stat", |args| {
        if args.is_empty() { return Err(PyError::type_error("stat() takes at least 1 argument")); }
        match std::fs::metadata(&crate::object::path_arg_to_string(&args[0])) {
            Ok(meta) => Ok(create_module("stat_result", stat_to_dict(&meta))),
            Err(e) => Err(PyError::OsError(format!("{}", e))),
        }
    });

    // --- os.lstat(path) ---
    os_func!("lstat", |args| {
        if args.is_empty() { return Err(PyError::type_error("lstat() takes at least 1 argument")); }
        match std::fs::symlink_metadata(&crate::object::path_arg_to_string(&args[0])) {
            Ok(meta) => Ok(create_module("stat_result", stat_to_dict(&meta))),
            Err(e) => Err(PyError::OsError(format!("{}", e))),
        }
    });

    // --- stat_result module with field index constants ---
    {
        let mut sr = HashMap::new();
        sr.insert_str("ST_MODE", py_int(0));
        sr.insert_str("ST_INO", py_int(1));
        sr.insert_str("ST_DEV", py_int(2));
        sr.insert_str("ST_NLINK", py_int(3));
        sr.insert_str("ST_UID", py_int(4));
        sr.insert_str("ST_GID", py_int(5));
        sr.insert_str("ST_SIZE", py_int(6));
        sr.insert_str("ST_ATIME", py_int(7));
        sr.insert_str("ST_MTIME", py_int(8));
        sr.insert_str("ST_CTIME", py_int(9));
        sr.insert_str("n_fields", py_int(10));
        sr.insert_str("n_sequence_fields", py_int(10));
        sr.insert_str("__doc__", py_str("stat_result: stat results as a module with named field indices"));
        d.insert_str("stat_result", create_module("stat_result", sr));
    }

    // --- os.chmod(path, mode) ---
    os_func!("chmod", |args| {
        if args.len() < 2 { return Err(PyError::type_error("chmod() takes at least 2 arguments")); }
        let path = crate::object::path_arg_to_string(&args[0]);
        let mode = args[1].as_i64().unwrap_or(0) as u32;
        use std::os::unix::fs::PermissionsExt;
        match std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)) {
            Ok(()) => Ok(py_none()),
            Err(e) => Err(PyError::OsError(format!("{}", e))),
        }
    });

    // --- os.chown(path, uid, gid) ---
    os_func!("chown", |args| {
        if args.len() < 3 { return Err(PyError::type_error("chown() takes at least 3 arguments")); }
        let path = args[0].str();
        let uid = args[1].as_i64().unwrap_or(-1);
        let gid = args[2].as_i64().unwrap_or(-1);
        use std::os::unix::fs::chown;
        match chown(
            &path,
            if uid == -1 { None } else { Some(uid as u32) },
            if gid == -1 { None } else { Some(gid as u32) },
        ) {
            Ok(()) => Ok(py_none()),
            Err(e) => Err(PyError::OsError(format!("{}", e))),
        }
    });

    // --- os.link(src, dst) ---
    os_func!("link", |args| {
        if args.len() < 2 { return Err(PyError::type_error("link() takes at least 2 arguments")); }
        match std::fs::hard_link(&args[0].str(), &args[1].str()) {
            Ok(()) => Ok(py_none()),
            Err(e) => Err(PyError::OsError(format!("{}", e))),
        }
    });

    // --- os.symlink(src, dst) ---
    os_func!("symlink", |args| {
        if args.len() < 2 { return Err(PyError::type_error("symlink() takes at least 2 arguments")); }
        use std::os::unix::fs::symlink;
        match symlink(&args[0].str(), &args[1].str()) {
            Ok(()) => Ok(py_none()),
            Err(e) => Err(PyError::OsError(format!("{}", e))),
        }
    });

    // --- os.readlink(path) ---
    os_func!("readlink", |args| {
        if args.is_empty() { return Err(PyError::type_error("readlink() takes at least 1 argument")); }
        match std::fs::read_link(&args[0].str()) {
            Ok(p) => Ok(py_str(&p.to_string_lossy())),
            Err(e) => Err(PyError::OsError(format!("{}", e))),
        }
    });

    // --- os.makedirs(path) ---
    os_func!("makedirs", |args| {
        if args.is_empty() { return Err(PyError::type_error("makedirs() takes at least 1 argument")); }
        let path = args[0].str();
        match std::fs::create_dir_all(&path) {
            Ok(()) => Ok(py_none()),
            Err(e) => Err(PyError::OsError(format!("{}", e))),
        }
    });

    // --- os.rmdir(path) ---
    os_func!("rmdir", |args| {
        if args.is_empty() { return Err(PyError::type_error("rmdir() takes at least 1 argument")); }
        match std::fs::remove_dir(&args[0].str()) {
            Ok(()) => Ok(py_none()),
            Err(e) => Err(PyError::OsError(format!("{}", e))),
        }
    });

    // --- os.walk(top): directory tree walker (returns list of tuples) ---
    os_func!("walk", |args| {
        if args.is_empty() { return Err(PyError::type_error("walk() takes at least 1 argument")); }
        let top = args[0].str();
        let mut results = Vec::new();
        let mut dirs_to_visit = vec![top];
        while let Some(dir) = dirs_to_visit.pop() {
            match std::fs::read_dir(&dir) {
                Ok(entries) => {
                    let mut dirname_strs: Vec<String> = Vec::new();
                    let mut filename_strs: Vec<String> = Vec::new();
                    for entry in entries.flatten() {
                        let name = entry.file_name().to_string_lossy().to_string();
                        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
                        if name == "." || name == ".." { continue; }
                        if is_dir {
                            dirname_strs.push(name);
                        } else {
                            filename_strs.push(name);
                        }
                    }
                    dirname_strs.sort();
                    filename_strs.sort();
                    let dirnames: Vec<PyObjectRef> = dirname_strs.iter().map(|s| py_str(s)).collect();
                    let filenames: Vec<PyObjectRef> = filename_strs.iter().map(|s| py_str(s)).collect();
                    results.push(py_tuple(vec![
                        py_str(&dir),
                        py_list(dirnames),
                        py_list(filenames),
                    ]));
                    // Push subdirs in reverse order for DFS alphabetical traversal
                    for dirname in dirname_strs.iter().rev() {
                        let sub = if dir.ends_with('/') {
                            format!("{}{}", dir, dirname)
                        } else {
                            format!("{}/{}", dir, dirname)
                        };
                        dirs_to_visit.push(sub);
                    }
                }
                Err(_) => { /* skip unreadable directories */ }
            }
        }
        Ok(PyObjectRef::new(PyObject::List(results)))
    });

    // --- File mode constants (from <sys/stat.h>) ---
    d.insert_str("S_IFMT", py_int(0o170000));
    d.insert_str("S_IFSOCK", py_int(0o140000));
    d.insert_str("S_IFLNK", py_int(0o120000));
    d.insert_str("S_IFREG", py_int(0o100000));
    d.insert_str("S_IFBLK", py_int(0o060000));
    d.insert_str("S_IFDIR", py_int(0o040000));
    d.insert_str("S_IFCHR", py_int(0o020000));
    d.insert_str("S_IFIFO", py_int(0o010000));
    d.insert_str("S_ISUID", py_int(0o4000));
    d.insert_str("S_ISGID", py_int(0o2000));
    d.insert_str("S_ISVTX", py_int(0o1000));
    d.insert_str("S_IRWXU", py_int(0o700));
    d.insert_str("S_IRUSR", py_int(0o400));
    d.insert_str("S_IWUSR", py_int(0o200));
    d.insert_str("S_IXUSR", py_int(0o100));
    d.insert_str("S_IRWXG", py_int(0o070));
    d.insert_str("S_IRGRP", py_int(0o040));
    d.insert_str("S_IWGRP", py_int(0o020));
    d.insert_str("S_IXGRP", py_int(0o010));
    d.insert_str("S_IRWXO", py_int(0o007));
    d.insert_str("S_IROTH", py_int(0o004));
    d.insert_str("S_IWOTH", py_int(0o002));
    d.insert_str("S_IXOTH", py_int(0o001));

    // OS constants needed by stdlib code
    d.insert_str("name", py_str("posix"));
    d.insert_str("sep", py_str("/"));
    d.insert_str("linesep", py_str("\n"));
    d.insert_str("pathsep", py_str(":"));

    // os.path sub-module will be wired as a proper submodule in vm.rs
    // The path attribute is set there (not inline) to allow proper os.path import
    d
}

/// Create the os.path submodule dict with path manipulation functions.
///
/// Provides: join, exists, isfile, isdir, abspath, dirname, basename,
/// splitext, split, getsize, getmtime, islink, expanduser, normpath, normcase
pub fn create_os_path_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    d.insert_str("supports_unicode_filenames", py_bool(!cfg!(windows)));
    macro_rules! path_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    d.insert_str("curdir", py_str("."));
    d.insert_str("pardir", py_str(".."));
    d.insert_str("sep", py_str(if cfg!(windows) { "\\" } else { "/" }));
    d.insert_str("altsep", if cfg!(windows) { py_str("/") } else { py_none() });
    d.insert_str("extsep", py_str("."));
    d.insert_str("pathsep", py_str(if cfg!(windows) { ";" } else { ":" }));
    d.insert_str("defpath", py_str(if cfg!(windows) { "." } else { ":/bin:/usr/bin" }));
    d.insert_str("devnull", py_str(if cfg!(windows) { "nul" } else { "/dev/null" }));

    // --- String-based path manipulation functions ---

    path_func!("join", |args| {
        let parts: Vec<String> = args.iter().map(|a| a.str()).collect();
        if parts.is_empty() { return Ok(py_str("")); }
        let result = parts.join("/");
        Ok(py_str(&result))
    });

    path_func!("dirname", |args| {
        if args.is_empty() { return Err(PyError::type_error("dirname() takes at least 1 argument")); }
        let path = args[0].str();
        let result = match path.rfind('/') {
            Some(i) => if i == 0 { "/".to_string() } else { path[..i].to_string() },
            None => ".".to_string(),
        };
        Ok(py_str(&result))
    });

    path_func!("basename", |args| {
        if args.is_empty() { return Err(PyError::type_error("basename() takes at least 1 argument")); }
        let path = args[0].str();
        let result = match path.rfind('/') {
            Some(i) => path[i+1..].to_string(),
            None => path,
        };
        Ok(py_str(&result))
    });

    path_func!("split", |args| {
        if args.is_empty() { return Err(PyError::type_error("split() takes at least 1 argument")); }
        let path = args[0].str();
        let (head, tail) = match path.rfind('/') {
            Some(i) => {
                let h = if i == 0 { "/".to_string() } else { path[..i].to_string() };
                let t = path[i+1..].to_string();
                (h, t)
            }
            None => (".".to_string(), path.clone()),
        };
        Ok(py_list(vec![py_str(&head), py_str(&tail)]))
    });

    path_func!("splitext", |args| {
        if args.is_empty() { return Err(PyError::type_error("splitext() takes at least 1 argument")); }
        let path = args[0].str();
        let dot = path.rfind('.');
        let (root, ext) = match dot {
            Some(i) if i > path.rfind('/').map_or(0, |j| j + 1) => {
                (path[..i].to_string(), path[i..].to_string())
            }
            _ => (path.clone(), "".to_string()),
        };
        Ok(py_list(vec![py_str(&root), py_str(&ext)]))
    });

    // --- Filesystem-based checks ---

    path_func!("exists", |args| {
        if args.is_empty() { return Err(PyError::type_error("exists() takes at least 1 argument")); }
        Ok(py_bool(std::path::Path::new(&args[0].str()).exists()))
    });

    path_func!("isfile", |args| {
        if args.is_empty() { return Err(PyError::type_error("isfile() takes at least 1 argument")); }
        Ok(py_bool(std::path::Path::new(&args[0].str()).is_file()))
    });

    path_func!("isdir", |args| {
        if args.is_empty() { return Err(PyError::type_error("isdir() takes at least 1 argument")); }
        Ok(py_bool(std::path::Path::new(&args[0].str()).is_dir()))
    });

    // `os.path.isabs(path)` — was missing entirely; a common, basic
    // path-classification check (does this path already start from the
    // filesystem root, or is it relative to somewhere).
    path_func!("isabs", |args| {
        if args.is_empty() { return Err(PyError::type_error("isabs() takes at least 1 argument")); }
        Ok(py_bool(std::path::Path::new(&crate::object::path_arg_to_string(&args[0])).is_absolute()))
    });

    // --- Path resolution and normalization ---

    path_func!("abspath", |args| {
        if args.is_empty() { return Err(PyError::type_error("abspath() takes at least 1 argument")); }
        let path_str = args[0].str();
        let path = std::path::Path::new(&path_str);
        if path.is_absolute() {
            // Resolve . and .. components for a clean absolute path
            let mut components: Vec<&str> = Vec::new();
            for c in path_str.split('/') {
                match c {
                    "." | "" => continue,
                    ".." => { components.pop(); }
                    c => { components.push(c); }
                }
            }
            let result = if components.is_empty() {
                "/".to_string()
            } else {
                format!("/{}", components.join("/"))
            };
            Ok(py_str(&result))
        } else {
            match std::env::current_dir() {
                Ok(cwd) => {
                    let abs = cwd.join(&path_str);
                    Ok(py_str(&abs.to_string_lossy().to_string()))
                }
                Err(e) => Err(PyError::OsError(format!("{}", e))),
            }
        }
    });

    // `os.path.realpath(path)` — resolves symlinks (via `std::fs::
    // canonicalize`) and returns an absolute path, falling back to the
    // plain `abspath`-style resolution above if the path doesn't exist
    // (real CPython's `realpath` doesn't require the path to exist either —
    // it resolves as much as it can and leaves the rest as-is). Missing
    // entirely before this — a common, general path-normalization idiom
    // real code reaches for constantly (not just a niche function).
    path_func!("realpath", |args| {
        if args.is_empty() { return Err(PyError::type_error("realpath() takes at least 1 argument")); }
        let path_str = crate::object::path_arg_to_string(&args[0]);
        match std::fs::canonicalize(&path_str) {
            Ok(resolved) => Ok(py_str(&resolved.to_string_lossy())),
            Err(_) => {
                // Path doesn't exist (or a component doesn't) — fall back
                // to plain absolute-path resolution without requiring
                // existence, matching real `realpath`'s graceful behavior.
                let path = std::path::Path::new(&path_str);
                if path.is_absolute() {
                    Ok(py_str(&path_str))
                } else {
                    match std::env::current_dir() {
                        Ok(cwd) => Ok(py_str(&cwd.join(&path_str).to_string_lossy())),
                        Err(e) => Err(PyError::OsError(format!("{}", e))),
                    }
                }
            }
        }
    });

    // --- Filesystem metadata ---

    path_func!("getsize", |args| {
        if args.is_empty() { return Err(PyError::type_error("getsize() takes at least 1 argument")); }
        match std::fs::metadata(&args[0].str()) {
            Ok(meta) => Ok(py_int(meta.len() as i64)),
            Err(e) => Err(PyError::OsError(format!("{}", e))),
        }
    });

    path_func!("getmtime", |args| {
        if args.is_empty() { return Err(PyError::type_error("getmtime() takes at least 1 argument")); }
        match std::fs::metadata(&args[0].str()) {
            Ok(meta) => {
                match meta.modified() {
                    Ok(time) => {
                        use std::time::SystemTime;
                        let duration = time.duration_since(SystemTime::UNIX_EPOCH)
                            .unwrap_or_default();
                        Ok(py_float(duration.as_secs_f64()))
                    }
                    Err(e) => Err(PyError::OsError(format!("{}", e))),
                }
            }
            Err(e) => Err(PyError::OsError(format!("{}", e))),
        }
    });

    path_func!("islink", |args| {
        if args.is_empty() { return Err(PyError::type_error("islink() takes at least 1 argument")); }
        match std::fs::symlink_metadata(&args[0].str()) {
            Ok(meta) => Ok(py_bool(meta.file_type().is_symlink())),
            Err(_) => Ok(py_bool(false)), // Python os.path.islink returns False on error
        }
    });

    // --- User expansion ---

    path_func!("expanduser", |args| {
        if args.is_empty() { return Err(PyError::type_error("expanduser() takes at least 1 argument")); }
        let path = args[0].str();
        if path == "~" || path.starts_with("~/") {
            match std::env::var("HOME") {
                Ok(home) => {
                    let result = if path == "~" {
                        home
                    } else {
                        format!("{}/{}", home, &path[2..])
                    };
                    Ok(py_str(&result))
                }
                Err(_) => {
                    Ok(py_str(&path))
                }
            }
        } else {
            Ok(py_str(&path))
        }
    });

    // --- Normalization ---

    path_func!("normpath", |args| {
        if args.is_empty() { return Err(PyError::type_error("normpath() takes at least 1 argument")); }
        let path = args[0].str();
        let mut parts: Vec<&str> = Vec::new();
        let is_absolute = path.starts_with('/');
        for segment in path.split('/') {
            match segment {
                "." | "" => continue,
                ".." => {
                    // Only pop if we won't go above root (for absolute paths)
                    // or if we have a regular component to pop (for relative)
                    if parts.is_empty() {
                        if !is_absolute {
                            parts.push("..");
                        }
                        // else: absolute path, just ignore (can't go above /)
                    } else if parts.last() == Some(&"..") {
                        parts.push("..");
                    } else {
                        parts.pop();
                    }
                }
                c => parts.push(c),
            }
        }
        let joined = parts.join("/");
        let result = if is_absolute {
            format!("/{}", joined)
        } else if joined.is_empty() {
            ".".to_string()
        } else {
            joined
        };
        Ok(py_str(&result))
    });

    path_func!("normcase", |args| {
        if args.is_empty() { return Err(PyError::type_error("normcase() takes at least 1 argument")); }
        let path = args[0].str();
        // On Unix, normcase is a no-op (returns path unchanged)
        // On Windows it would lowercase and convert / to \\
        Ok(py_str(&path))
    });

    // commonprefix(list) — longest literal (character-wise, not
    // path-component-aware) string prefix shared by every path in `list`.
    // Was missing entirely — needed by the real `unittest.util` module
    // (`from os.path import commonprefix`, used for diffing assertion
    // messages), which is itself needed by any real `unittest`-based test
    // suite (Django's own test framework included).
    path_func!("commonprefix", |args| {
        if args.is_empty() { return Err(PyError::type_error("commonprefix() takes at least 1 argument")); }
        let paths: Vec<String> = crate::object::collect_iterable(&args[0])?
            .iter().map(|p| p.str()).collect();
        if paths.is_empty() { return Ok(py_str("")); }
        let first = &paths[0];
        let mut prefix_len = first.chars().count();
        for p in &paths[1..] {
            let common = first.chars().zip(p.chars()).take_while(|(a, b)| a == b).count();
            prefix_len = prefix_len.min(common);
        }
        let prefix: String = first.chars().take(prefix_len).collect();
        Ok(py_str(&prefix))
    });

    d
}

/// Looks up `name` on `obj` the same way the VM's own `LOAD_ATTR` opcode
/// does — as opposed to the raw `get_attribute()` free function, which does
/// NOT auto-bind. Two real gaps this closes for any caller (like
/// `attrgetter`/`methodcaller` below) that resolves an attribute
/// PROGRAMMATICALLY rather than through the opcode:
/// (1) a user-defined `Instance`'s own method: `get_attribute` alone returns
/// the raw, UNBOUND `Function` — calling it directly skips `self` entirely,
/// binding whatever the caller's first real argument was to `self` instead
/// (confirmed: `operator.methodcaller('greet', 'world')` on an instance
/// raised `NameError: local variable 'name' referenced before assignment`,
/// because `'world'` silently became `self` and the real `name` parameter
/// was never filled at all).
/// (2) a NATIVE type's method (e.g. `"hello".upper`): these are built with
/// `self_obj: PyObject::None` as a documented PLACEHOLDER meaning "rebind me
/// to whatever object I was actually looked up on" — a rebind step ONLY
/// `LOAD_ATTR`'s own inline copy performs. Skipping it means the returned
/// `BuiltinMethod` keeps `self_obj = None` forever, so calling it later
/// operates on `None` instead of the real object (confirmed:
/// `operator.attrgetter('upper')("hello")()` returned `'NONE'` — the
/// uppercased string representation of `None`, not `"hello"`'s real
/// `.upper()` result `'HELLO'`).
fn bound_attr(obj: &PyObjectRef, name: &str) -> PyResult<PyObjectRef> {
    if matches!(&*obj.borrow(), PyObject::Instance { .. }) {
        if let Ok(Some(bound)) = with_vm_mut(|vm| vm.resolve_descriptor_attr(obj, name)) {
            return Ok(bound);
        }
    }
    let attr = obj.borrow().get_attribute(name)?;
    let needs_rebind = matches!(&*attr.borrow(), PyObject::BuiltinMethod { self_obj, .. } if matches!(&*self_obj.borrow(), PyObject::None));
    if needs_rebind {
        if let PyObject::BuiltinMethod { name: n, func, .. } = &*attr.borrow() {
            return Ok(PyObjectRef::imm(PyObject::BuiltinMethod { name: n.clone(), func: *func, self_obj: obj.clone() }));
        }
    }
    Ok(attr)
}

pub fn create_operator_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! op_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

    op_func!("add", |args| {
        if args.len() < 2 { return Err(PyError::type_error("operator.add requires 2 arguments")); }
        py_add(&args[0], &args[1])
    });
    op_func!("sub", |args| {
        if args.len() < 2 { return Err(PyError::type_error("operator.sub requires 2 arguments")); }
        py_sub(&args[0], &args[1])
    });
    op_func!("mul", |args| {
        if args.len() < 2 { return Err(PyError::type_error("operator.mul requires 2 arguments")); }
        py_mul(&args[0], &args[1])
    });
    op_func!("truediv", |args| {
        if args.len() < 2 { return Err(PyError::type_error("operator.truediv requires 2 arguments")); }
        py_div(&args[0], &args[1])
    });
    op_func!("floordiv", |args| {
        if args.len() < 2 { return Err(PyError::type_error("operator.floordiv requires 2 arguments")); }
        py_floor_div(&args[0], &args[1])
    });
    op_func!("mod", |args| {
        if args.len() < 2 { return Err(PyError::type_error("operator.mod requires 2 arguments")); }
        py_mod(&args[0], &args[1])
    });
    op_func!("pow", |args| {
        if args.len() < 2 { return Err(PyError::type_error("operator.pow requires 2 arguments")); }
        py_pow(&args[0], &args[1])
    });
    op_func!("lt", |args| {
        if args.len() < 2 { return Err(PyError::type_error("operator.lt requires 2 arguments")); }
        py_compare(&args[0], &args[1], 0)
    });
    op_func!("le", |args| {
        if args.len() < 2 { return Err(PyError::type_error("operator.le requires 2 arguments")); }
        py_compare(&args[0], &args[1], 1)
    });
    op_func!("eq", |args| {
        if args.len() < 2 { return Err(PyError::type_error("operator.eq requires 2 arguments")); }
        py_compare(&args[0], &args[1], 2)
    });
    op_func!("ne", |args| {
        if args.len() < 2 { return Err(PyError::type_error("operator.ne requires 2 arguments")); }
        py_compare(&args[0], &args[1], 5)
    });
    op_func!("ge", |args| {
        if args.len() < 2 { return Err(PyError::type_error("operator.ge requires 2 arguments")); }
        py_compare(&args[0], &args[1], 3)
    });
    op_func!("gt", |args| {
        if args.len() < 2 { return Err(PyError::type_error("operator.gt requires 2 arguments")); }
        py_compare(&args[0], &args[1], 4)
    });
    op_func!("and_", |args| {
        if args.len() < 2 { return Err(PyError::type_error("operator.and_ requires 2 arguments")); }
        py_bit_and(&args[0], &args[1])
    });
    op_func!("or_", |args| {
        if args.len() < 2 { return Err(PyError::type_error("operator.or_ requires 2 arguments")); }
        py_bit_or(&args[0], &args[1])
    });
    op_func!("xor", |args| {
        if args.len() < 2 { return Err(PyError::type_error("operator.xor requires 2 arguments")); }
        py_bit_xor(&args[0], &args[1])
    });
    op_func!("not_", |args| {
        if args.is_empty() { return Err(PyError::type_error("operator.not_ requires 1 argument")); }
        Ok(py_not(&args[0]))
    });
    op_func!("getitem", |args| {
        if args.len() < 2 { return Err(PyError::type_error("operator.getitem requires 2 arguments")); }
        py_getitem(&args[0], &args[1])
    });
    op_func!("setitem", |args| {
        if args.len() < 3 { return Err(PyError::type_error("operator.setitem requires 3 arguments")); }
        py_setitem(&args[0], &args[1], args[2].clone())?;
        Ok(py_none())
    });
    op_func!("delitem", |args| {
        if args.len() < 2 { return Err(PyError::type_error("operator.delitem requires 2 arguments")); }
        py_delitem(&args[0], &args[1])?;
        Ok(py_none())
    });
    op_func!("contains", |args| {
        if args.len() < 2 { return Err(PyError::type_error("operator.contains requires 2 arguments")); }
        py_contains(&args[0], &args[1])
    });
    op_func!("index", |args| {
        if args.is_empty() { return Err(PyError::type_error("operator.index requires 1 argument")); }
        to_index(&args[0]).map(|i| py_int(i))
    });
    op_func!("indexOf", |args| {
        if args.len() < 2 { return Err(PyError::type_error("operator.indexOf requires 2 arguments")); }
        let it = crate::object::builtin_iter(&[args[0].clone()])?;
        let mut idx: i64 = 0;
        loop {
            match crate::object::builtin_next(&[it.clone()]) {
                Ok(v) => {
                    if crate::object::py_compare(&v, &args[1], 2)?.truthy() { return Ok(py_int(idx)); }
                    idx += 1;
                }
                Err(PyError::StopIteration) => return Err(PyError::value_error("sequence.index(x): x not in sequence")),
                Err(e) => return Err(e),
            }
        }
    });
    op_func!("countOf", |args| {
        if args.len() < 2 { return Err(PyError::type_error("operator.countOf requires 2 arguments")); }
        let it = crate::object::builtin_iter(&[args[0].clone()])?;
        let mut count: i64 = 0;
        loop {
            match crate::object::builtin_next(&[it.clone()]) {
                Ok(v) => { if crate::object::py_compare(&v, &args[1], 2)?.truthy() { count += 1; } }
                Err(PyError::StopIteration) => return Ok(py_int(count)),
                Err(e) => return Err(e),
            }
        }
    });
    op_func!("truth", |args| {
        if args.is_empty() { return Err(PyError::type_error("operator.truth requires 1 argument")); }
        Ok(py_bool(args[0].truthy()))
    });
    op_func!("neg", |args| {
        if args.is_empty() { return Err(PyError::type_error("operator.neg requires 1 argument")); }
        py_neg(&args[0])
    });
    op_func!("pos", |args| {
        if args.is_empty() { return Err(PyError::type_error("operator.pos requires 1 argument")); }
        Ok(args[0].clone())
    });
    op_func!("abs", |args| {
        if args.is_empty() { return Err(PyError::type_error("operator.abs requires 1 argument")); }
        if let Some(i) = args[0].as_i64() { return Ok(py_int(i.abs())); }
        if let Some(f) = args[0].as_f64() { return Ok(py_float(f.abs())); }
        let obj = args[0].borrow();
        match &*obj {
            PyObject::Int(n) => Ok(py_int(n.clone().abs())),
            PyObject::Float(n) => Ok(py_float(n.abs())),
            _ => Err(PyError::type_error(format!("bad operand type for abs(): '{}'", obj.type_name()))),
        }
    });
    op_func!("inv", |args| {
        if args.is_empty() { return Err(PyError::type_error("operator.inv requires 1 argument")); }
        if let Some(i) = args[0].as_i64() { return Ok(py_int(!i)); }
        let obj = args[0].borrow();
        if let PyObject::Int(n) = &*obj { Ok(py_int(!n.clone())) }
        else { Err(PyError::type_error(format!("bad operand type for inv(): '{}'", obj.type_name()))) }
    });
    op_func!("lshift", |args| {
        if args.len() < 2 { return Err(PyError::type_error("operator.lshift requires 2 arguments")); }
        py_lshift(&args[0], &args[1])
    });
    op_func!("rshift", |args| {
        if args.len() < 2 { return Err(PyError::type_error("operator.rshift requires 2 arguments")); }
        py_rshift(&args[0], &args[1])
    });
    op_func!("length_hint", |args| {
        if args.is_empty() { return Err(PyError::type_error("operator.length_hint requires 1 argument")); }
        builtin_len(args)
    });
    // `operator.is_`/`is_not` — plain identity checks, real Python's
    // function-object equivalents of the `is`/`is not` operators (used
    // e.g. as a `key=`/comparison callable where a bare operator won't do).
    // Missing entirely before.
    op_func!("is_", |args| {
        if args.len() < 2 { return Err(PyError::type_error("operator.is_ requires 2 arguments")); }
        Ok(py_bool(args[0].is(&args[1])))
    });
    op_func!("is_not", |args| {
        if args.len() < 2 { return Err(PyError::type_error("operator.is_not requires 2 arguments")); }
        Ok(py_bool(!args[0].is(&args[1])))
    });
    // __iadd__ etc. — just wrap the binop
    op_func!("__add__", |args| { if args.len() < 2 { return Err(PyError::type_error("__add__ requires 2 arguments")); } py_add(&args[0], &args[1]) });
    op_func!("__sub__", |args| { if args.len() < 2 { return Err(PyError::type_error("__sub__ requires 2 arguments")); } py_sub(&args[0], &args[1]) });
    op_func!("__mul__", |args| { if args.len() < 2 { return Err(PyError::type_error("__mul__ requires 2 arguments")); } py_mul(&args[0], &args[1]) });
    op_func!("__truediv__", |args| { if args.len() < 2 { return Err(PyError::type_error("__truediv__ requires 2 arguments")); } py_div(&args[0], &args[1]) });
    op_func!("__floordiv__", |args| { if args.len() < 2 { return Err(PyError::type_error("__floordiv__ requires 2 arguments")); } py_floor_div(&args[0], &args[1]) });
    op_func!("__mod__", |args| { if args.len() < 2 { return Err(PyError::type_error("__mod__ requires 2 arguments")); } py_mod(&args[0], &args[1]) });
    op_func!("__pow__", |args| { if args.len() < 2 { return Err(PyError::type_error("__pow__ requires 2 arguments")); } py_pow(&args[0], &args[1]) });
    op_func!("__and__", |args| { if args.len() < 2 { return Err(PyError::type_error("__and__ requires 2 arguments")); } py_bit_and(&args[0], &args[1]) });
    op_func!("__or__", |args| { if args.len() < 2 { return Err(PyError::type_error("__or__ requires 2 arguments")); } py_bit_or(&args[0], &args[1]) });
    op_func!("__xor__", |args| { if args.len() < 2 { return Err(PyError::type_error("__xor__ requires 2 arguments")); } py_bit_xor(&args[0], &args[1]) });
    op_func!("__lshift__", |args| { if args.len() < 2 { return Err(PyError::type_error("__lshift__ requires 2 arguments")); } py_lshift(&args[0], &args[1]) });
    op_func!("__rshift__", |args| { if args.len() < 2 { return Err(PyError::type_error("__rshift__ requires 2 arguments")); } py_rshift(&args[0], &args[1]) });
    op_func!("__getitem__", |args| { if args.len() < 2 { return Err(PyError::type_error("__getitem__ requires 2 arguments")); } py_getitem(&args[0], &args[1]) });
    op_func!("__setitem__", |args| { if args.len() < 3 { return Err(PyError::type_error("__setitem__ requires 3 arguments")); } py_setitem(&args[0], &args[1], args[2].clone())?; Ok(py_none()) });

    // itemgetter factory
    d.insert_str("itemgetter", PyObjectRef::new(PyObject::BuiltinFunction {
        name: "itemgetter".to_string(),
        func: |args| {
            if args.is_empty() { return Err(PyError::type_error("itemgetter requires at least 1 argument")); }
            let items = args.to_vec();
            // Return a callable that does getitem on its argument
            let getter = PyObjectRef::new(PyObject::Closure(Rc::new(move |get_args| {
                if get_args.is_empty() { return Err(PyError::type_error("itemgetter called with no arguments")); }
                let obj = &get_args[0];
                if items.len() == 1 {
                    py_getitem(obj, &items[0])
                } else {
                    let mut results = Vec::new();
                    for item in &items {
                        results.push(py_getitem(obj, item)?);
                    }
                    Ok(PyObjectRef::imm(PyObject::Tuple(results)))
                }
            })));
            Ok(getter)
        },
    }));

    // attrgetter factory
    d.insert_str("attrgetter", PyObjectRef::new(PyObject::BuiltinFunction {
        name: "attrgetter".to_string(),
        func: |args| {
            if args.is_empty() { return Err(PyError::type_error("attrgetter requires at least 1 argument")); }
            let attrs: Vec<String> = args.iter().map(|a| a.str()).collect();
            let getter = PyObjectRef::new(PyObject::Closure(Rc::new(move |get_args| {
                if get_args.is_empty() { return Err(PyError::type_error("attrgetter called with no arguments")); }
                if attrs.len() == 1 {
                    bound_attr(&get_args[0], &attrs[0])
                } else {
                    let mut results = Vec::new();
                    for attr in &attrs {
                        results.push(bound_attr(&get_args[0], attr)?);
                    }
                    Ok(PyObjectRef::imm(PyObject::Tuple(results)))
                }
            })));
            Ok(getter)
        },
    }));

    // `operator.methodcaller(name, *args)` — missing entirely. Returns a
    // callable that, given `obj`, calls `obj.name(*args)` — a common
    // `key=`/callback idiom (`sorted(objs, key=methodcaller('lower'))`,
    // real trigger: CPython's own `test_operator.py`). Positional args only
    // (no keyword-argument support) — good enough for the common case, and
    // consistent with this module's existing `itemgetter`/`attrgetter`
    // factories just above, neither of which support keywords either.
    d.insert_str("methodcaller", PyObjectRef::new(PyObject::BuiltinFunction {
        name: "methodcaller".to_string(),
        func: |args| {
            if args.is_empty() { return Err(PyError::type_error("methodcaller requires at least 1 argument")); }
            let method_name = args[0].str();
            let extra_args: Vec<PyObjectRef> = args[1..].to_vec();
            let caller = PyObjectRef::new(PyObject::Closure(Rc::new(move |call_args| {
                if call_args.is_empty() { return Err(PyError::type_error("methodcaller's callable requires an argument")); }
                let obj = &call_args[0];
                let method = bound_attr(obj, &method_name)?;
                let mut full_args = extra_args.clone();
                full_args.extend_from_slice(&call_args[1..]);
                builtin_call(&method, &full_args)
            })));
            Ok(caller)
        },
    }));

    // `operator.__all__` — missing entirely (`AttributeError`), breaking
    // even the module's own `test___all__` sanity check at collection time
    // (real trigger: CPython's own `test_operator.py`). Computed from the
    // dict's own already-public (non-dunder) keys rather than a hand-
    // maintained literal list, so it can't drift out of sync with whatever
    // this function actually defines above.
    let all_names: Vec<PyObjectRef> = d.keys().filter(|k| !k.starts_with('_')).map(|k| py_str(k)).collect();
    d.insert_str("__all__", py_list(all_names));

    d
}

use std::rc::Rc;
use num_traits::ToPrimitive;

/// Native __future__ module: defines _Feature tuples and feature flags.
pub fn create_future_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();

    // _Feature helper: tuples of (flag, name, first_release, optional_since)
    let feature = |flag: i64, name: &str, first: &str, optional: &str| -> PyObjectRef {
        PyObjectRef::imm(PyObject::Tuple(vec![
            py_int(flag),
            py_str(name),
            py_str(first),
            py_str(optional),
        ]))
    };

    d.insert_str("nested_scopes", feature(0x01, "nested_scopes", "2.1.0", "2.2.0"));
    d.insert_str("generators", feature(0x02, "generators", "2.2.0", "2.3.0"));
    d.insert_str("division", feature(0x04, "division", "2.2.0", "3.0.0"));
    d.insert_str("absolute_import", feature(0x08, "absolute_import", "2.5.0", "3.0.0"));
    d.insert_str("with_statement", feature(0x10, "with_statement", "2.5.0", "2.6.0"));
    d.insert_str("print_function", feature(0x20, "print_function", "2.6.0", "3.0.0"));
    d.insert_str("unicode_literals", feature(0x40, "unicode_literals", "2.6.0", "3.0.0"));
    d.insert_str("barry_as_FLUFL", feature(0x80, "barry_as_FLUFL", "3.1.0", "4.0.0"));
    d.insert_str("generator_stop", feature(0x100, "generator_stop", "3.5.0", "3.7.0"));
    d.insert_str("annotations", feature(0x200, "annotations", "3.7.0", "3.11.0"));

    d.insert_str("all_feature_names", py_list(vec![
        py_str("nested_scopes"), py_str("generators"), py_str("division"),
        py_str("absolute_import"), py_str("with_statement"), py_str("print_function"),
        py_str("unicode_literals"), py_str("barry_as_FLUFL"), py_str("generator_stop"),
        py_str("annotations"),
    ]));

    d.insert_str("__doc__", py_str("Future feature statements (from __future__)"));
    d.insert_str("__name__", py_str("__future__"));
    d.insert_str("__package__", py_str(""));
    d
}

/// Native errno module — POSIX error code constants
pub fn create_errno_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    // Standard POSIX errno codes used by tempfile and os modules
    d.insert_str("EPERM", py_int(1));
    d.insert_str("ENOENT", py_int(2));
    d.insert_str("ESRCH", py_int(3));
    d.insert_str("EINTR", py_int(4));
    d.insert_str("EIO", py_int(5));
    d.insert_str("ENXIO", py_int(6));
    d.insert_str("E2BIG", py_int(7));
    d.insert_str("ENOEXEC", py_int(8));
    d.insert_str("EBADF", py_int(9));
    d.insert_str("ECHILD", py_int(10));
    d.insert_str("EAGAIN", py_int(11));
    d.insert_str("ENOMEM", py_int(12));
    d.insert_str("EACCES", py_int(13));
    d.insert_str("EFAULT", py_int(14));
    d.insert_str("ENOTBLK", py_int(15));
    d.insert_str("EBUSY", py_int(16));
    d.insert_str("EEXIST", py_int(17));
    d.insert_str("EXDEV", py_int(18));
    d.insert_str("ENODEV", py_int(19));
    d.insert_str("ENOTDIR", py_int(20));
    d.insert_str("EISDIR", py_int(21));
    d.insert_str("EINVAL", py_int(22));
    d.insert_str("ENFILE", py_int(23));
    d.insert_str("EMFILE", py_int(24));
    d.insert_str("ENOTTY", py_int(25));
    d.insert_str("ETXTBSY", py_int(26));
    d.insert_str("EFBIG", py_int(27));
    d.insert_str("ENOSPC", py_int(28));
    d.insert_str("ESPIPE", py_int(29));
    d.insert_str("EROFS", py_int(30));
    d.insert_str("EMLINK", py_int(31));
    d.insert_str("EPIPE", py_int(32));
    d.insert_str("EDOM", py_int(33));
    d.insert_str("ERANGE", py_int(34));
    d.insert_str("ENOSYS", py_int(38));
    d.insert_str("EOPNOTSUPP", py_int(95));
    d.insert_str("__name__", py_str("errno"));
    d
}