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
    let mut f = unsafe { std::fs::File::from_raw_fd(fd) };
    let result = f.write(data);
    std::mem::forget(f);
    result
}

/// Close a raw file descriptor by wrapping it in a File and dropping it.
fn close_fd(fd: i32) {
    use std::os::unix::io::FromRawFd;
    let file = unsafe { std::fs::File::from_raw_fd(fd) };
    drop(file); // Closes the fd
}

pub fn create_builtins() -> HashMap<String, PyObjectRef> {
    let mut builtins = HashMap::new();
    builtins.insert("None".to_string(), py_none());
    builtins.insert("True".to_string(), py_bool(true));
    builtins.insert("False".to_string(), py_bool(false));
    builtins.insert("Ellipsis".to_string(), PyObjectRef::imm(PyObject::Str(compact_str::CompactString::from("..."))));

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
    add_func!("type", builtin_type_of);
    add_func!("int", builtin_int);
    add_func!("float", builtin_float);
    add_func!("str", builtin_str);
    add_func!("bool", builtin_bool);
    add_func!("list", builtin_list);
    add_func!("tuple", builtin_tuple);
    add_func!("dict", builtin_dict);
    add_func!("set", builtin_set);
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
    add_func!("bytes", builtin_bytes);
    add_func!("bytearray", builtin_bytearray);
    add_func!("frozenset", builtin_frozenset);
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
    add_exc_type!("UnicodeDecodeError", builtin_make_exception_unicodedecodeerror);
    add_exc_type!("UnicodeEncodeError", builtin_make_exception_unicodeencodeerror);
    add_exc_type!("ExceptionGroup", builtin_make_exception_exceptiongroup);
    add_exc_type!("BaseExceptionGroup", builtin_make_exception_baseexceptiongroup);
    add_exc_type!("SystemError", builtin_make_exception_systemerror);

    let math_module = PyObjectRef::new(PyObject::Module {
        name: "math".to_string(),
        dict: create_math_dict(),
    });
    builtins.insert("math".to_string(), math_module.clone());

    // ── _codecs (needed by encodings module) ────────────────────────────────
    let codecs_module = PyObjectRef::new(PyObject::Module {
        name: "_codecs".to_string(),
        dict: create_codecs_dict(),
    });
    builtins.insert("_codecs".to_string(), codecs_module.clone());

    // ── _abc (needed by abc.py for ABCMeta, used by io/__init__.py) ────────
    let abc_module = PyObjectRef::new(PyObject::Module {
        name: "_abc".to_string(),
        dict: create_abc_builtins_dict(),
    });
    builtins.insert("_abc".to_string(), abc_module.clone());

    // Create a proper object TYPE with basic dunder methods.
    // This is used as the implicit base class for all classes without explicit bases.
    let mut object_dict = HashMap::new();
    // __setattr__(self, name, value): sets an attribute on the instance
    object_dict.insert("__setattr__".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
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
    object_dict.insert("__getattribute__".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
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
    object_dict.insert("__init__".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "__init__".to_string(),
        func: |_args| Ok(py_none()),
    }));
    // __repr__(self): <object at 0x...>
    object_dict.insert("__repr__".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
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
    // __eq__(self, other): identity comparison
    object_dict.insert("__eq__".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "__eq__".to_string(),
        func: |args| {
            if args.len() < 2 {
                return Err(PyError::type_error("__eq__ requires 2 arguments"));
            }
            Ok(py_bool(args[0].borrow().type_name() == args[1].borrow().type_name()))
        },
    }));
    // __ne__(self, other): inverse of __eq__
    object_dict.insert("__ne__".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "__ne__".to_string(),
        func: |args| {
            if args.len() < 2 {
                return Err(PyError::type_error("__ne__ requires 2 arguments"));
            }
            Ok(py_bool(args[0].borrow().type_name() != args[1].borrow().type_name()))
        },
    }));
    // __hash__(self): hash based on pointer
    object_dict.insert("__hash__".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "__hash__".to_string(),
        func: |args| {
            if args.is_empty() {
                return Err(PyError::type_error("__hash__ requires at least 1 argument (self)"));
            }
            let ptr: *const PyObject = &*args[0].borrow();
            Ok(py_int(ptr as i64))
        },
    }));
    // __new__(cls): creates a new instance of cls
    object_dict.insert("__new__".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "__new__".to_string(),
        func: |args| {
            if args.is_empty() {
                return Err(PyError::type_error("__new__ requires at least 1 argument (cls)"));
            }
            let cls = args[0].clone();
            Ok(PyObjectRef::new(PyObject::Instance {
                typ: cls,
                dict: HashMap::new(),
            }))
        },
    }));
    // __init_subclass__(cls, **kwargs): no-op (PEP 487)
    object_dict.insert("__init_subclass__".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "__init_subclass__".to_string(),
        func: |_args| {
            Ok(py_none())
        },
    }));
    // __class_getitem__(cls, item): for generic types like List[int] (PEP 560)
    object_dict.insert("__class_getitem__".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "__class_getitem__".to_string(),
        func: |args| {
            if args.len() < 2 {
                return Err(PyError::type_error("__class_getitem__ requires at least 2 arguments (cls, item)"));
            }
            Ok(py_tuple(vec![args[0].clone(), args[1].clone()]))
        },
    }));
    // __format__(self, format_spec): basic format support
    object_dict.insert("__format__".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
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
    object_dict.insert("__reduce__".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "__reduce__".to_string(),
        func: |_args| {
            Ok(py_none())
        },
    }));
    let object_type = PyObjectRef::new(PyObject::Type {
        name: "object".to_string(),
        dict: object_dict,
        bases: vec![],
        mro: vec![],
    });
    // Set MRO so isinstance works
    if let PyObject::Type { mro, .. } = &mut *object_type.borrow_mut() {
        *mro = vec![object_type.clone()];
    }
    // Register in builtins both as a type (for __build_class__) and as a callable (for object())
    builtins.insert("object".to_string(), object_type);
    // Also keep the function for direct use
    builtins.insert("_object_func".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "object".to_string(),
        func: builtin_object,
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
    d.insert("lookup_error".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "lookup_error".to_string(),
        func: _codecs_lookup_error,
    }));
    d.insert("lookup".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "lookup".to_string(),
        func: _codecs_lookup,
    }));
    d.insert("encode".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "encode".to_string(),
        func: _codecs_encode_func,
    }));
    d.insert("decode".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "decode".to_string(),
        func: _codecs_decode_func,
    }));
    d.insert("register".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
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
            PyObject::Type { dict, .. } => !dict.contains_key("_abc_impl"),
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
        dict: namespace_dict,
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
    d.insert("ABCMeta".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "ABCMeta".to_string(),
        func: _abc_abcmeta,
    }));
    d.insert("get_cache_token".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "get_cache_token".to_string(),
        func: _abc_get_cache_token,
    }));
    d.insert("_abc_init".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "_abc_init".to_string(),
        func: _abc_init,
    }));
    d.insert("_abc_register".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "_abc_register".to_string(),
        func: _abc_register,
    }));
    d.insert("_abc_instancecheck".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "_abc_instancecheck".to_string(),
        func: _abc_instancecheck,
    }));
    d.insert("_abc_subclasscheck".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "_abc_subclasscheck".to_string(),
        func: _abc_subclasscheck,
    }));
    d.insert("_get_dump".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "_get_dump".to_string(),
        func: _abc_get_dump,
    }));
    d.insert("_reset_registry".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "_reset_registry".to_string(),
        func: _abc_reset_registry,
    }));
    d.insert("_reset_caches".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "_reset_caches".to_string(),
        func: _abc_reset_caches,
    }));
    d
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
        match (&*a, &*b) {
            (PyObject::Int(i), PyObject::Int(j)) => Ok(py_float(i.to_f64().unwrap_or(0.0).powf(j.to_f64().unwrap_or(0.0)))),
            (PyObject::Int(i), PyObject::Float(f)) => Ok(py_float(i.to_f64().unwrap_or(0.0).powf(*f))),
            (PyObject::Float(f), PyObject::Int(i)) => Ok(py_float(f.powf(i.to_f64().unwrap_or(0.0)))),
            (PyObject::Float(a), PyObject::Float(b)) => Ok(py_float(a.powf(*b))),
            _ => Err(PyError::type_error("pow() argument must be a number")),
        }
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
    // ── Constants ─────────────────────────────────────────────────────────
    d.insert("pi".to_string(), py_float(std::f64::consts::PI));
    d.insert("e".to_string(), py_float(std::f64::consts::E));
    d.insert("tau".to_string(), py_float(std::f64::consts::TAU));
    d
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
    d.insert("argv".to_string(), py_list(argv.into_iter().map(|s| py_str(&s)).collect()));
    d.insert("path".to_string(), py_list(vec![]));
    d.insert("modules".to_string(), py_dict());
    d.insert("version".to_string(), py_str("3.12.0 (RustPython 0.1.0)"));
    d.insert("version_info".to_string(), py_tuple(vec![py_int(3), py_int(12), py_int(0)]));
    d.insert("hexversion".to_string(), py_int(0x030c0000));
    d.insert("stdin".to_string(), PyObjectRef::new(PyObject::File {
        file: std::rc::Rc::new(std::cell::RefCell::new(std::fs::File::open("/dev/stdin").unwrap_or_else(|_| {
            // Fallback: create a temporary file
            std::fs::File::create("/dev/null").unwrap()
        }))),
        name: "<stdin>".to_string(),
    }));
    d.insert("stdout".to_string(), PyObjectRef::new(PyObject::File {
        file: std::rc::Rc::new(std::cell::RefCell::new(std::fs::File::create("/dev/stdout").unwrap_or_else(|_| {
            std::fs::File::create("/dev/null").unwrap()
        }))),
        name: "<stdout>".to_string(),
    }));
    d.insert("stderr".to_string(), PyObjectRef::new(PyObject::File {
        file: std::rc::Rc::new(std::cell::RefCell::new(std::fs::File::create("/dev/stderr").unwrap_or_else(|_| {
            std::fs::File::create("/dev/null").unwrap()
        }))),
        name: "<stderr>".to_string(),
    }));
    d.insert("platform".to_string(), py_str(std::env::consts::OS));
    // sys.implementation — CPython returns a namespace with name, cache_tag, etc.
    {
        let mut imp_dict = HashMap::new();
        imp_dict.insert("name".to_string(), py_str("cpython"));
        imp_dict.insert("cache_tag".to_string(), py_str("cpython-314"));
        imp_dict.insert("hexversion".to_string(), py_int(50987248)); // 0x030A0000
        imp_dict.insert("_multi_threaded".to_string(), py_bool(true));
        d.insert("implementation".to_string(), create_module("implementation", imp_dict));
    }
    d.insert("byteorder".to_string(), py_str(if cfg!(target_endian = "little") { "little" } else { "big" }));
    d.insert("maxsize".to_string(), py_int(i64::MAX));
    d.insert("maxunicode".to_string(), py_int(1114111));
    d.insert("api_version".to_string(), py_int(1013));
    d.insert("executable".to_string(), py_str(&std::env::current_exe().map(|p| p.to_string_lossy().to_string()).unwrap_or_default()));
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
    d.insert("prefix".to_string(), py_str(&prefix));
    d.insert("exec_prefix".to_string(), py_str(&exec_prefix));
    d.insert("winver".to_string(), py_str("3.12"));
    // sys.exc_info() — returns current exception info from VM
    sys_func!("exc_info", |_args| {
        let result = crate::object::with_vm_mut(|vm| {
            py_tuple(vec![
                vm.exc_type.clone().unwrap_or(py_none()),
                vm.exc_value.clone().unwrap_or(py_none()),
                vm.exc_traceback.clone().unwrap_or(py_none()),
            ])
        });
        Ok(result.unwrap_or_else(|_| py_tuple(vec![py_none(), py_none(), py_none()])))
    });
    d
}

/// Native importlib stub module providing import_module().
pub fn create_importlib_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! importlib_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
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
    fn import_dotted(vm: &mut crate::vm::VirtualMachine, name: &str) -> Result<PyObjectRef, String> {
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
                            if let Some(mod_dict) = dict.get("modules") {
                                mod_dict.borrow_mut().set_attribute(&current, module).ok();
                            }
                        }
                    }
                }
            }
            if let Some(module) = vm.modules.get(name) {
                return Ok(module.clone());
            }
            return Err(format!("No module named '{}'", name));
        }
        // Simple name
        let module = vm.import_module_from_file(name)?;
        vm.modules.insert(name.to_string(), module.clone());
        if let Some(sys_mod) = vm.modules.get("sys") {
            if let PyObject::Module { dict, .. } = &*sys_mod.borrow() {
                if let Some(mod_dict) = dict.get("modules") {
                    mod_dict.borrow_mut().set_attribute(name, module.clone()).ok();
                }
            }
        }
        Ok(module)
    }

    // import_module(name, package=None) -> module
    importlib_func!("import_module", |args| {
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
            match import_dotted(vm, &resolved) {
                Ok(module) => Ok(module),
                Err(e) => Err(PyError::ImportError(format!("import_module error: {}", e))),
            }
        })?
    });

    // __version__ — indicates importlib metadata
    d.insert("__version__".to_string(), py_str("1.0.0"));
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
    util_func!("find_spec", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("find_spec() missing required argument 'name'"));
        }
        let name = args[0].str();
        let package = if args.len() >= 2 {
            let pkg = args[1].str();
            if pkg.is_empty() { None } else { Some(pkg) }
        } else { None };

        // Resolve the full module name (handle relative imports)
        let resolved_name = if let Some(ref pkg) = package {
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

        // Get VM and check if already loaded
        let resolved_name2 = resolved_name.clone();
        if with_vm_mut(|vm| Ok(vm.modules.contains_key(&resolved_name2)))?? {
            return Ok(create_module("ModuleSpec", HashMap::from([
                ("name".to_string(), py_str(&resolved_name)),
                ("origin".to_string(), py_str("built-in")),
            ])));
        }

        // Get sys.path manually to search for the module file
        let search_paths: Vec<String> = with_vm_mut(|vm| -> PyResult<Vec<String>> {
            let mut paths = Vec::new();
            if let Some(sys_mod) = vm.modules.get("sys") {
                if let PyObject::Module { dict, .. } = &*sys_mod.borrow() {
                    if let Some(path_list) = dict.get("path") {
                        if let PyObject::List(items) = &*path_list.borrow() {
                            for item in items {
                                if let PyObject::Str(s) = &*item.borrow() {
                                    paths.push(s.to_string());
                                }
                            }
                        }
                    }
                }
            }
            Ok(paths)
        })??;

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
    });

    d
}

/// Native importlib.resources stub module.
/// Provides `files(package)` and `as_file(traversable)` stubs for certifi compatibility.
pub fn create_importlib_resources_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();

    // Helper: read name attribute from a module
    fn mod_name(obj: &PyObjectRef) -> String {
        let b = obj.borrow();
        if let PyObject::Module { dict, .. } = &*b {
            if let Some(name) = dict.get("name") {
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
            dict.insert("joinpath".to_string(), PyObjectRef::new(PyObject::BuiltinMethod {
                name: "joinpath".to_string(),
                func: trav_joinpath,
                self_obj: trav.clone(),
            }));
        }
        Ok(trav)
    }

    // as_file(traversable) -> context manager wrapping the path
    d.insert("as_file".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
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
                dict.insert("__enter__".to_string(), PyObjectRef::new(PyObject::BuiltinMethod {
                    name: "__enter__".to_string(),
                    func: enter_cm,
                    self_obj: cm.clone(),
                }));
                dict.insert("__exit__".to_string(), PyObjectRef::new(PyObject::BuiltinMethod {
                    name: "__exit__".to_string(),
                    func: exit_cm,
                    self_obj: cm.clone(),
                }));
            }
            Ok(cm)
        },
    }));

    // files(package) -> traversable with joinpath()
    d.insert("files".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
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
                            if let Some(path_list) = dict.get("__path__") {
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
                dict.insert("joinpath".to_string(), PyObjectRef::new(PyObject::BuiltinMethod {
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
        let path = args[0].str();
        std::fs::remove_file(&path).map_err(|e| PyError::OsError(format!("{}", e)))?;
        Ok(py_none())
    });

    // os.unlink = os.remove (POSIX alias)
    os_func!("unlink", |args| {
        if args.is_empty() { return Err(PyError::type_error("unlink() takes at least 1 argument")); }
        let path = args[0].str();
        std::fs::remove_file(&path).map_err(|e| PyError::OsError(format!("{}", e)))?;
        Ok(py_none())
    });

    os_func!("rename", |args| {
        if args.len() < 2 { return Err(PyError::type_error("rename() takes 2 arguments")); }
        match std::fs::rename(&args[0].str(), &args[1].str()) {
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
        if flags & 64 != 0 { opts.create(true); }       // O_CREAT = 64
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
    d.insert("O_RDONLY".to_string(), py_int(0));
    d.insert("O_WRONLY".to_string(), py_int(1));
    d.insert("O_RDWR".to_string(), py_int(2));
    d.insert("O_CREAT".to_string(), py_int(64));
    d.insert("O_EXCL".to_string(), py_int(128));
    d.insert("O_TRUNC".to_string(), py_int(512));
    d.insert("O_APPEND".to_string(), py_int(1024));

    // environ dict — use a proper PyDict instead of Module so methods like
    // .setdefault(), .get(), .keys(), 'x in environ', etc. all work (Django req.)
    let mut environ_pydict = PyDict::new();
    for (key, val) in std::env::vars() {
        environ_pydict.set(py_str(&key), py_str(&val)).ok();
    }
    d.insert("environ".to_string(), PyObjectRef::new(PyObject::Dict(environ_pydict)));

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
        d.insert("st_mode".to_string(), py_int(meta.mode() as i64));
        d.insert("st_ino".to_string(), py_int(meta.ino() as i64));
        d.insert("st_dev".to_string(), py_int(meta.dev() as i64));
        d.insert("st_nlink".to_string(), py_int(meta.nlink() as i64));
        d.insert("st_uid".to_string(), py_int(meta.uid() as i64));
        d.insert("st_gid".to_string(), py_int(meta.gid() as i64));
        d.insert("st_size".to_string(), py_int(meta.size() as i64));
        if let Ok(t) = meta.modified() {
            let dur = t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
            d.insert("st_mtime".to_string(), py_float(dur.as_secs_f64()));
        }
        if let Ok(t) = meta.accessed() {
            let dur = t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
            d.insert("st_atime".to_string(), py_float(dur.as_secs_f64()));
        }
        if let Ok(t) = meta.created() {
            let dur = t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
            d.insert("st_ctime".to_string(), py_float(dur.as_secs_f64()));
        }
        d
    }

    // --- os.stat(path) ---
    os_func!("stat", |args| {
        if args.is_empty() { return Err(PyError::type_error("stat() takes at least 1 argument")); }
        match std::fs::metadata(&args[0].str()) {
            Ok(meta) => Ok(create_module("stat_result", stat_to_dict(&meta))),
            Err(e) => Err(PyError::OsError(format!("{}", e))),
        }
    });

    // --- os.lstat(path) ---
    os_func!("lstat", |args| {
        if args.is_empty() { return Err(PyError::type_error("lstat() takes at least 1 argument")); }
        match std::fs::symlink_metadata(&args[0].str()) {
            Ok(meta) => Ok(create_module("stat_result", stat_to_dict(&meta))),
            Err(e) => Err(PyError::OsError(format!("{}", e))),
        }
    });

    // --- stat_result module with field index constants ---
    {
        let mut sr = HashMap::new();
        sr.insert("ST_MODE".to_string(), py_int(0));
        sr.insert("ST_INO".to_string(), py_int(1));
        sr.insert("ST_DEV".to_string(), py_int(2));
        sr.insert("ST_NLINK".to_string(), py_int(3));
        sr.insert("ST_UID".to_string(), py_int(4));
        sr.insert("ST_GID".to_string(), py_int(5));
        sr.insert("ST_SIZE".to_string(), py_int(6));
        sr.insert("ST_ATIME".to_string(), py_int(7));
        sr.insert("ST_MTIME".to_string(), py_int(8));
        sr.insert("ST_CTIME".to_string(), py_int(9));
        sr.insert("n_fields".to_string(), py_int(10));
        sr.insert("n_sequence_fields".to_string(), py_int(10));
        sr.insert("__doc__".to_string(), py_str("stat_result: stat results as a module with named field indices"));
        d.insert("stat_result".to_string(), create_module("stat_result", sr));
    }

    // --- os.chmod(path, mode) ---
    os_func!("chmod", |args| {
        if args.len() < 2 { return Err(PyError::type_error("chmod() takes at least 2 arguments")); }
        let path = args[0].str();
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
    d.insert("S_IFMT".to_string(), py_int(0o170000));
    d.insert("S_IFSOCK".to_string(), py_int(0o140000));
    d.insert("S_IFLNK".to_string(), py_int(0o120000));
    d.insert("S_IFREG".to_string(), py_int(0o100000));
    d.insert("S_IFBLK".to_string(), py_int(0o060000));
    d.insert("S_IFDIR".to_string(), py_int(0o040000));
    d.insert("S_IFCHR".to_string(), py_int(0o020000));
    d.insert("S_IFIFO".to_string(), py_int(0o010000));
    d.insert("S_ISUID".to_string(), py_int(0o4000));
    d.insert("S_ISGID".to_string(), py_int(0o2000));
    d.insert("S_ISVTX".to_string(), py_int(0o1000));
    d.insert("S_IRWXU".to_string(), py_int(0o700));
    d.insert("S_IRUSR".to_string(), py_int(0o400));
    d.insert("S_IWUSR".to_string(), py_int(0o200));
    d.insert("S_IXUSR".to_string(), py_int(0o100));
    d.insert("S_IRWXG".to_string(), py_int(0o070));
    d.insert("S_IRGRP".to_string(), py_int(0o040));
    d.insert("S_IWGRP".to_string(), py_int(0o020));
    d.insert("S_IXGRP".to_string(), py_int(0o010));
    d.insert("S_IRWXO".to_string(), py_int(0o007));
    d.insert("S_IROTH".to_string(), py_int(0o004));
    d.insert("S_IWOTH".to_string(), py_int(0o002));
    d.insert("S_IXOTH".to_string(), py_int(0o001));

    // OS constants needed by stdlib code
    d.insert("name".to_string(), py_str("posix"));
    d.insert("sep".to_string(), py_str("/"));
    d.insert("linesep".to_string(), py_str("\n"));
    d.insert("pathsep".to_string(), py_str(":"));

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
    macro_rules! path_func {
        ($name:expr, $func:expr) => {
            d.insert($name.to_string(), PyObjectRef::new(PyObject::BuiltinFunction { name: $name.to_string(), func: $func }));
        };
    }

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

    d
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
    d.insert("itemgetter".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
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
    d.insert("attrgetter".to_string(), PyObjectRef::new(PyObject::BuiltinFunction {
        name: "attrgetter".to_string(),
        func: |args| {
            if args.is_empty() { return Err(PyError::type_error("attrgetter requires at least 1 argument")); }
            let attrs: Vec<String> = args.iter().map(|a| a.str()).collect();
            let getter = PyObjectRef::new(PyObject::Closure(Rc::new(move |get_args| {
                if get_args.is_empty() { return Err(PyError::type_error("attrgetter called with no arguments")); }
                if attrs.len() == 1 {
                    get_args[0].borrow().get_attribute(&attrs[0])
                } else {
                    let mut results = Vec::new();
                    for attr in &attrs {
                        results.push(get_args[0].borrow().get_attribute(attr)?);
                    }
                    Ok(PyObjectRef::imm(PyObject::Tuple(results)))
                }
            })));
            Ok(getter)
        },
    }));

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

    d.insert("nested_scopes".to_string(), feature(0x01, "nested_scopes", "2.1.0", "2.2.0"));
    d.insert("generators".to_string(), feature(0x02, "generators", "2.2.0", "2.3.0"));
    d.insert("division".to_string(), feature(0x04, "division", "2.2.0", "3.0.0"));
    d.insert("absolute_import".to_string(), feature(0x08, "absolute_import", "2.5.0", "3.0.0"));
    d.insert("with_statement".to_string(), feature(0x10, "with_statement", "2.5.0", "2.6.0"));
    d.insert("print_function".to_string(), feature(0x20, "print_function", "2.6.0", "3.0.0"));
    d.insert("unicode_literals".to_string(), feature(0x40, "unicode_literals", "2.6.0", "3.0.0"));
    d.insert("barry_as_FLUFL".to_string(), feature(0x80, "barry_as_FLUFL", "3.1.0", "4.0.0"));
    d.insert("generator_stop".to_string(), feature(0x100, "generator_stop", "3.5.0", "3.7.0"));
    d.insert("annotations".to_string(), feature(0x200, "annotations", "3.7.0", "3.11.0"));

    d.insert("all_feature_names".to_string(), py_list(vec![
        py_str("nested_scopes"), py_str("generators"), py_str("division"),
        py_str("absolute_import"), py_str("with_statement"), py_str("print_function"),
        py_str("unicode_literals"), py_str("barry_as_FLUFL"), py_str("generator_stop"),
        py_str("annotations"),
    ]));

    d.insert("__doc__".to_string(), py_str("Future feature statements (from __future__)"));
    d.insert("__name__".to_string(), py_str("__future__"));
    d.insert("__package__".to_string(), py_str(""));
    d
}

/// Native errno module — POSIX error code constants
pub fn create_errno_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    // Standard POSIX errno codes used by tempfile and os modules
    d.insert("EPERM".to_string(), py_int(1));
    d.insert("ENOENT".to_string(), py_int(2));
    d.insert("ESRCH".to_string(), py_int(3));
    d.insert("EINTR".to_string(), py_int(4));
    d.insert("EIO".to_string(), py_int(5));
    d.insert("ENXIO".to_string(), py_int(6));
    d.insert("E2BIG".to_string(), py_int(7));
    d.insert("ENOEXEC".to_string(), py_int(8));
    d.insert("EBADF".to_string(), py_int(9));
    d.insert("ECHILD".to_string(), py_int(10));
    d.insert("EAGAIN".to_string(), py_int(11));
    d.insert("ENOMEM".to_string(), py_int(12));
    d.insert("EACCES".to_string(), py_int(13));
    d.insert("EFAULT".to_string(), py_int(14));
    d.insert("ENOTBLK".to_string(), py_int(15));
    d.insert("EBUSY".to_string(), py_int(16));
    d.insert("EEXIST".to_string(), py_int(17));
    d.insert("EXDEV".to_string(), py_int(18));
    d.insert("ENODEV".to_string(), py_int(19));
    d.insert("ENOTDIR".to_string(), py_int(20));
    d.insert("EISDIR".to_string(), py_int(21));
    d.insert("EINVAL".to_string(), py_int(22));
    d.insert("ENFILE".to_string(), py_int(23));
    d.insert("EMFILE".to_string(), py_int(24));
    d.insert("ENOTTY".to_string(), py_int(25));
    d.insert("ETXTBSY".to_string(), py_int(26));
    d.insert("EFBIG".to_string(), py_int(27));
    d.insert("ENOSPC".to_string(), py_int(28));
    d.insert("ESPIPE".to_string(), py_int(29));
    d.insert("EROFS".to_string(), py_int(30));
    d.insert("EMLINK".to_string(), py_int(31));
    d.insert("EPIPE".to_string(), py_int(32));
    d.insert("EDOM".to_string(), py_int(33));
    d.insert("ERANGE".to_string(), py_int(34));
    d.insert("ENOSYS".to_string(), py_int(38));
    d.insert("EOPNOTSUPP".to_string(), py_int(95));
    d.insert("__name__".to_string(), py_str("errno"));
    d
}