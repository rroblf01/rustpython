// Split out of the former monolithic object.rs (see the file-splitting
// refactor's memory entry for context) — this file holds native-base
// subclassing support (`class Foo(list): ...`, `class Foo(dict): ...`,
// `class Foo(str): ...`, etc.).
use super::*;

// ---- Native-base subclassing (`class Foo(list): ...`, `class Foo(dict): ...`, `class Foo(str): ...`) ----
//
// list/dict/str are PyObject::BuiltinFunction constructors, not real
// PyObject::Type classes — they have no bases/mro/dict of their own. Rather
// than adding a struct field to PyObject::Type/Instance (both are
// constructed/destructured in 200+ places across this codebase, making a
// shape change enormously invasive), the "this class transparently wraps a
// native list/dict/str" fact is recorded as a plain entry in the class's
// own (already-mutable) dict, and each instance's native payload is a real,
// independently addressable PyObjectRef stored under a reserved key in the
// instance's own (already-mutable) attribute dict. Because that payload is
// a genuine PyObject::List/Dict/Str, all of list/dict/str's existing method
// implementations work on it completely unchanged — they just need to be
// reached via delegation when a subclass doesn't override them.

pub(crate) const NATIVE_BASE_MARKER: &str = "__native_base__";
pub(crate) const NATIVE_BACKING_KEY: &str = "__native__";
/// Internal bookkeeping key (not user-visible in `__dict__`/introspection,
/// same treatment as NATIVE_BASE_MARKER) recording the *metaclass* used to
/// build a class object, when it's something other than the plain builtin
/// `type` — e.g. a class built via `class Choices(Enum, metaclass=ChoicesType)`
/// carries `metatype = ChoicesType` here. Our object model has no separate
/// "type of this type" field on `PyObject::Type` (classes ARE
/// `PyObject::Type` regardless of what constructed them), so this dict entry
/// is how `type(cls)`, metaclass-level attribute fallback, and metaclass
/// *inheritance* (a subclass with no explicit `metaclass=` must still use its
/// base's custom metaclass) all recover "which metaclass built this".
pub(crate) const METATYPE_KEY: &str = "__metatype__";
/// Internal bookkeeping key (same treatment as the other markers above)
/// marking a `PyObject::Type` as a REAL native value type (`int`, and
/// eventually `str`/`list`/`dict`/`float`/etc.) rather than an ordinary
/// Python-defined class — the codebase's long-standing "native types
/// aren't real Type objects" architecture gap, being closed one type at a
/// time. Points at the type's original native constructor
/// `PyObject::BuiltinFunction` (e.g. `builtin_int`). `call_function`'s
/// generic `Type`-construction path checks for this key FIRST: if present,
/// it calls the constructor directly and returns its raw, UNWRAPPED result
/// (a plain `PyObject::Int`, never a `PyObject::Instance`) — `int(5)` must
/// return a raw int, not an instance-of-int wrapper. A user subclass
/// (`class MyInt(int): ...`) is unaffected: `default_build_class`'s
/// native-base detection recognizes this key as an alternative shape of
/// "native base" alongside the existing `BuiltinFunction`-based check, and
/// routes subclass construction through the existing, unchanged
/// `NATIVE_BASE_MARKER`/`NATIVE_BACKING_KEY` machinery instead.
pub(crate) const NATIVE_VALUE_CTOR_KEY: &str = "__native_value_ctor__";

/// The metaclass that built `typ`, if it's something other than plain
/// `type` — checked on the class's own dict only, exactly like
/// `native_base_of_type` (this key is not propagated to subclasses; each
/// subclass gets its own METATYPE_KEY set at its own construction time by
/// `__build_class__`'s metaclass-inheritance resolution).
pub(crate) fn metatype_of(typ: &PyObjectRef) -> Option<PyObjectRef> {
    if let PyObject::Type { dict, .. } = &*typ.borrow() {
        dict.get_str(METATYPE_KEY).cloned()
    } else {
        None
    }
}

pub(crate) fn is_recognized_native_base_name(name: &str) -> bool {
    matches!(
        name,
        "list"
            | "dict"
            | "str"
            | "int"
            | "float"
            | "tuple"
            | "bytes"
            | "set"
            | "complex"
            | "bytearray"
            | "frozenset"
            | "deque"
    )
}

/// True iff `name` is one of the builtin exception "classes" registered by
/// `create_builtins`'s `add_exc_type!` macro (`core.rs`) — these are
/// represented as plain `PyObject::BuiltinFunction`s in this codebase (not
/// `PyObject::Type`s), the same representation used for ordinary native
/// utility functions. That shared representation is exactly why a
/// `BuiltinFunction` found via a class's own dict must NOT be blanket
/// auto-bound as an instance method on generic attribute access: doing so
/// is correct for a genuine native method (e.g. `hmac`'s `HMAC.hexdigest`,
/// which deliberately relies on `self` being auto-prepended) but wrong for
/// a class attribute holding one of these exception constructors (e.g.
/// `failureException = AssertionError`) — real Python never binds a class
/// reference just because it's accessed via `self.attr`. Checked by NAME
/// (not a fixed list of function pointers) since it must match whatever
/// `add_exc_type!` registered under that exact name, and the two need to
/// stay in sync as a matter of course (adding a new exception type there
/// without a matching entry here would silently regress to the old
/// wrong-auto-bind behavior for that one type only — low risk, since new
/// entries are rare and this list is co-located for easy eyeballing).
pub(crate) fn is_builtin_exception_class_name(name: &str) -> bool {
    matches!(
        name,
        "BaseException" | "Exception" | "TypeError" | "ValueError" |
        "ZeroDivisionError" | "NameError" | "UnboundLocalError" | "AttributeError" | "IndexError" |
        "KeyError" | "RuntimeError" | "StopIteration" | "AssertionError" |
        "OSError" | "ImportError" | "LookupError" | "ArithmeticError" |
        "FloatingPointError" | "OverflowError" | "EnvironmentError" | "IOError" |
        "FileNotFoundError" | "PermissionError" | "NotImplementedError" |
        "RecursionError" | "KeyboardInterrupt" | "GeneratorExit" | "SystemExit" |
        "ModuleNotFoundError" | "StopAsyncIteration" | "EOFError" | "SyntaxError" |
        "PythonFinalizationError" |
        "EncodingWarning" |
        "ConnectionError" | "BrokenPipeError" | "ConnectionRefusedError" |
        "BlockingIOError" | "ChildProcessError" | "InterruptedError" |
        "TimeoutError" | "UnicodeDecodeError" | "UnicodeEncodeError" |
        "ExceptionGroup" | "BaseExceptionGroup" | "SystemError" | "Warning" |
        "UserWarning" | "DeprecationWarning" | "PendingDeprecationWarning" |
        "SyntaxWarning" | "RuntimeWarning" | "FutureWarning" | "ImportWarning" |
        "UnicodeWarning" | "BytesWarning" | "ResourceWarning" | "ReferenceError" |
        "BufferError" | "MemoryError" | "NotADirectoryError" | "IsADirectoryError" |
        "FileExistsError" | "ConnectionAbortedError" | "ConnectionResetError" |
        "ProcessLookupError" | "UnicodeTranslateError" | "IndentationError" | "TabError" |
        // `UnicodeError` itself (as opposed to its `UnicodeDecodeError`/
        // `UnicodeEncodeError` subclasses, both already listed above) was
        // missing despite being registered the same way via `add_exc_type!`
        // — found via the same struct/decimal/pickle audit below.
        "UnicodeError" |
        // Module-specific exception classes (each defined the same way —
        // a bare `PyObject::BuiltinFunction` whose closure builds a
        // `PyObject::Exception` — but registered on their OWN module's dict
        // rather than via `add_exc_type!`/`create_builtins`, so they need
        // separate entries here). Real trigger: `binascii.Error` failing
        // `isinstance(binascii.Error, type)` (found via CPython's own
        // `test_base64.py`, whose `self.assertRaises(binascii.Error, ...)`
        // calls unittest's `_is_subtype`, which requires this to be True —
        // same root gap as the core-builtin case documented above, just for
        // a name outside that fixed core list).
        "Error" | "InvalidStateError" | "CertificateError" | "SSLError" | "OperationalError" |
        // `struct.error` — lowercase, matching real CPython's `struct.error`
        // attribute name exactly (unlike most other module exceptions here,
        // which use a capitalized class name). Found via `test_struct.py`'s
        // own `assertRaisesRegex(struct.error, ...)` calls.
        "error" |
        // `pickle` module exceptions (`pickle.PickleError`/`PicklingError`/
        // `UnpicklingError`) and `graphlib.CycleError` — same audit pass.
        "CycleError" | "PickleError" | "PicklingError" | "UnpicklingError" |
        // `decimal` module's signal/exception classes (`decimal.DecimalException`
        // and its subclasses) — same audit pass, found by cross-referencing
        // every `builtin_make_exception_*` constructor registered anywhere
        // in `src/modules/` against this list.
        "DecimalException" | "InvalidOperation" | "DivisionByZero" | "Inexact" | "Rounded" |
        "Clamped" | "Overflow" | "Underflow" | "FloatOperation"
    )
}

/// Name of the builtin type (list/dict/str) a class transparently
/// subclasses, if any — checked on the class's own dict only (the marker is
/// propagated down into every subclass's own dict at class-creation time,
/// so this does not need to walk mro/bases itself).
pub(crate) fn native_base_of_type(typ: &PyObjectRef) -> Option<String> {
    if let PyObject::Type { dict, .. } = &*typ.borrow() {
        dict.get_str(NATIVE_BASE_MARKER).map(|v| v.str())
    } else {
        None
    }
}

/// Name of the closest built-in exception constructor (`Exception`,
/// `ValueError`, ...) a class's ancestry reaches, if any — walked directly
/// (not via a propagated marker like `native_base_of_type`, since built-in
/// exception "classes" are `PyObject::BuiltinFunction`s, not
/// `PyObject::Type`s, so they never appear in `mro` at all — only each real
/// ancestor `Type`'s own direct `bases` field can reference one). Used to
/// detect `class MyError(Exception): pass`-style user exception subclasses
/// (at any depth: `class MoreSpecific(MyError): pass` too) that don't
/// override `__init__`, which still need `self.args` populated exactly like
/// calling `Exception(*args)` directly would (see the call site in
/// `vm.rs`'s Type-instantiation logic).
pub(crate) fn find_exception_base_name(typ: &PyObjectRef) -> Option<String> {
    let (bases, mro): (Vec<PyObjectRef>, Vec<PyObjectRef>) = {
        if let PyObject::Type { bases, mro, .. } = &*typ.borrow() {
            (bases.clone(), mro.clone())
        } else {
            return None;
        }
    };
    let mut base_lists: Vec<Vec<PyObjectRef>> = vec![bases];
    for m in &mro {
        if let PyObject::Type { bases: b, .. } = &*m.borrow() {
            base_lists.push(b.clone());
        }
    }
    for base_list in base_lists {
        for b in base_list {
            if let PyObject::BuiltinFunction { name, .. } = &*b.borrow() {
                if crate::vm::is_exception_subclass(name, "BaseException") {
                    return Some(name.clone());
                }
            }
        }
    }
    None
}

/// The native backing value (a real, independent PyObject::List/Dict/Str)
/// for an instance of a native-subclassing class, if any.
pub(crate) fn native_backing_of(obj: &PyObjectRef) -> Option<PyObjectRef> {
    if let PyObject::Instance { dict, .. } = &*obj.borrow() {
        dict.get(NATIVE_BACKING_KEY).cloned()
    } else {
        None
    }
}

/// Builds an instance of `typ` carrying `backing` as its native backing —
/// used to wrap a computed native value back into the original subclass
/// (e.g. `-IntSubclass(5)` is an IntSubclass with backing -5).
pub(crate) fn make_subclass_instance(typ: &PyObjectRef, backing: PyObjectRef) -> PyObjectRef {
    let mut dict = crate::object::AttrMap::new();
    dict.insert(NATIVE_BACKING_KEY.to_string(), backing);
    PyObjectRef::new(PyObject::Instance {
        typ: typ.clone(),
        dict,
    })
}

pub(crate) fn make_native_backing(kind: &str) -> PyObjectRef {
    match kind {
        "list" => py_list(vec![]),
        "dict" => py_dict(),
        "str" => py_str(""),
        "int" => py_int(0),
        "float" => py_float(0.0),
        "tuple" => py_tuple(vec![]),
        "bytes" => PyObjectRef::imm(PyObject::Bytes(Vec::new())),
        "set" => py_set(),
        "complex" => PyObjectRef::imm(PyObject::Complex(0.0, 0.0)),
        "bytearray" => PyObjectRef::new(PyObject::ByteArray(Vec::new())),
        "frozenset" => PyObjectRef::imm(PyObject::FrozenSet(PySet::new())),
        "deque" => py_deque(std::collections::VecDeque::new(), None),
        _ => py_none(),
    }
}

/// Mimics list(iterable)/dict(...)/str(x) construction for a class that
/// transparently subclasses a native type and doesn't override __init__.
/// Returns the populated native backing (callers replace the instance's
/// NATIVE_BACKING_KEY entry with it, rather than mutating in place, since
/// the existing value's representation — e.g. an inline SmallStr — may not
/// even be back-referenceable via borrow_mut()).
pub(crate) fn synthesize_native_init(
    kind: &str,
    args: &[PyObjectRef],
    keywords: &[(String, PyObjectRef)],
) -> PyResult<PyObjectRef> {
    match kind {
        "list" => {
            // `subclass(sequence=())` for a `class subclass(list)` must
            // TypeError (test_list::test_keywords_in_subclass), not silently
            // treat the kwargs dict as the iterable.
            if !keywords.is_empty() {
                return Err(PyError::type_error("list() takes no keyword arguments"));
            }
            if let Some(first) = args.first() {
                if matches!(&*first.borrow(), PyObject::Dict(_)) {
                    return Err(PyError::type_error("list() takes no keyword arguments"));
                }
                Ok(py_list(collect_iterable(first)?))
            } else {
                Ok(py_list(vec![]))
            }
        }
        "dict" => {
            // Mirrors `call_function`'s own generic "pack keywords into a
            // trailing dict positional arg" convention (`vm.rs`) — needed
            // here too since `dict(**kwargs)`/`MyDict(a=1, b=2)` (a
            // native-base subclass with no explicit `__init__`) must
            // populate its backing from KEYWORD args exactly like the
            // plain `dict(...)` builtin does. Without this, `keywords` was
            // silently dropped entirely (this function never received them
            // at all), so `class MyDict(dict): pass; MyDict(a=1, b=2)`
            // produced an empty dict instead of `{'a': 1, 'b': 2}`.
            if args.is_empty() && keywords.is_empty() {
                Ok(py_dict())
            } else if keywords.is_empty() {
                builtin_dict(args)
            } else {
                let mut kw_dict = PyDict::new();
                for (k, v) in keywords {
                    kw_dict.set(py_str(k), v.clone())?;
                }
                let mut combined = args.to_vec();
                combined.push(PyObjectRef::new(PyObject::Dict(Box::new(kw_dict))));
                builtin_dict(&combined)
            }
        }
        "str" => {
            if let Some(v) = args.first() {
                Ok(py_str(&v.str()))
            } else {
                Ok(py_str(""))
            }
        }
        "int" => builtin_int(args),
        "float" => builtin_float(args),
        "tuple" => {
            if let Some(iterable) = args.first() {
                Ok(py_tuple(collect_iterable(iterable)?))
            } else {
                Ok(py_tuple(vec![]))
            }
        }
        "bytes" => builtin_bytes(args),
        "set" => builtin_set(args),
        "complex" => builtin_complex(args),
        "bytearray" => builtin_bytearray(args),
        "frozenset" => builtin_frozenset(args),
        "deque" => {
            let mut combined = args.to_vec();
            if !keywords.is_empty() {
                let mut kw_dict = PyDict::new();
                for (k, v) in keywords {
                    kw_dict.set(py_str(k), v.clone())?;
                }
                combined.push(PyObjectRef::new(PyObject::Dict(Box::new(kw_dict))));
            }
            builtin_deque(&combined)
        }
        _ => Ok(py_none()),
    }
}

/// Real `dict.__init__`/`list.__init__`/etc. for a class that transparently
/// subclasses a native type — installed directly in each migrated type's own
/// dict (see `modules::core::create_core_dicts`'s per-type `*_dict` blocks)
/// so it's found by the NORMAL mro walk, both for `super().__init__(...)`
/// calls and for the unbound `dict.__init__(instance, ...)` idiom.
///
/// Before this existed, NEITHER call form actually populated anything:
/// mro order for e.g. `class Foo(dict)` is `[Foo, dict, object]`, and since
/// `dict`'s own type-dict had NO `__init__` entry at all, the mro walk
/// (in `get_attribute_impl`'s `super()`-handling arm, `attrs.rs`) fell
/// through PAST `dict` to `object`'s generic no-op `__init__` and stopped
/// there — `object`'s `__init__` was found and auto-bound first, so a
/// SEPARATE special-case closure written specifically to populate the
/// native backing (also in `attrs.rs`, guarded on "not found via any Type
/// in the mro") was silently unreachable dead code: the mro walk always
/// found `object.__init__` before ever falling through to it. Confirmed via
/// `_strptime.py`'s `TimeRE.__init__` (`class TimeRE(dict): ... base =
/// super(); base.__init__(mapping)`), which builds a completely EMPTY dict
/// instead of the real strptime-directive regex table, breaking every
/// `datetime.strptime`/`time.strptime` call — SIGSEGV/hang-adjacent
/// discovery via `test_datetime.py` unittest module loading (a `KeyError`
/// deep in `_strptime`, since the resulting `TimeRE` instance was empty).
///
/// A plain `BuiltinFunction` (not `BuiltinMethod` with a placeholder
/// `self_obj` — the recurring arg-shift bug documented throughout this
/// session) taking `self` as `args[0]`, matching the exact unbound-method
/// convention real `def __init__(self, ...)` methods use: the normal mro
/// walk auto-binds `Function`/`BuiltinFunction` values found via `super()`
/// into a `BoundMethod` (prepending `self` at CALL time), and a direct
/// `dict.__init__(instance, ...)` access returns it unbound, requiring the
/// caller to pass `instance` explicitly — both forms end up calling this
/// with `self` in `args[0]` either way.
pub(crate) fn native_base_init_builtin(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let self_obj = args
        .first()
        .ok_or_else(|| PyError::type_error("__init__() missing required argument: 'self'"))?
        .clone();
    let rest = &args[1..];
    let kind = {
        let typ = if let PyObject::Instance { typ, .. } = &*self_obj.borrow() {
            Some(typ.clone())
        } else {
            None
        };
        typ.and_then(|t| native_base_of_type(&t))
    };
    if let Some(kind) = kind {
        let native = synthesize_native_init(&kind, rest, &[])?;
        if let PyObject::Instance { dict, .. } = &mut *self_obj.borrow_mut() {
            dict.insert(NATIVE_BACKING_KEY.to_string(), native);
        }
    }
    Ok(py_none())
}

pub(crate) fn collect_iterable(iterable: &PyObjectRef) -> PyResult<Vec<PyObjectRef>> {
    let iter_obj = builtin_iter(&[iterable.clone()])?;
    let mut items = Vec::new();
    loop {
        match builtin_next(&[iter_obj.clone()]) {
            Ok(v) => items.push(v),
            Err(PyError::StopIteration) => break,
            Err(e) => return Err(e),
        }
    }
    Ok(items)
}

pub(crate) fn get_instance_slots(typ: &PyObjectRef) -> Option<Vec<String>> {
    let typ_ref = typ.borrow();
    if let PyObject::Type {
        dict: type_dict,
        mro,
        ..
    } = &*typ_ref
    {
        let mut all_slots = Vec::new();

        // Check the type's own __slots__
        if let Some(slots_val) = type_dict.get_str("__slots__") {
            extract_slots(slots_val, &mut all_slots);
        }

        // Check bases' __slots__ (skip self at index 0)
        for base in mro.iter().skip(1) {
            let base_ref = base.borrow();
            if let PyObject::Type {
                dict: base_dict, ..
            } = &*base_ref
            {
                if let Some(slots_val) = base_dict.get_str("__slots__") {
                    extract_slots(slots_val, &mut all_slots);
                }
            }
        }

        if !all_slots.is_empty() {
            // If `__dict__` is among the declared slots (real CPython's
            // `class C: __slots__ = ('x', 'y', '__dict__')`), the instance
            // carries a dict and accepts ARBITRARY attributes — there is no
            // effective restriction to enforce (real trigger: a deque
            // subclass with `__slots__ = ('x', 'y', '__dict__')` setting a
            // plain `z` attribute).
            if all_slots.iter().any(|s| s == "__dict__") {
                return None;
            }
            return Some(all_slots);
        }
    }
    None
}

/// Get the class name for an Instance's type, used for error messages.
pub(crate) fn get_type_name_for_instance(typ: &PyObjectRef) -> String {
    let typ_ref = typ.borrow();
    if let PyObject::Type { name, .. } = &*typ_ref {
        name.clone()
    } else {
        "object".to_string()
    }
}
