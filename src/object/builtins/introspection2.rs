use super::*;
use super::introspection1::{abc_registry_matches_in_subtree, call_bound_method, IsinstanceRecursionGuard};

pub fn builtin_isinstance(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let trace = std::env::var("RPY_TRACE_IS").is_ok();
    if trace {
        let o_t = args[0].borrow().type_name();
        let c_t = args[1].borrow().type_name();
        let c_meta = crate::object::metatype_of(&args[1])
            .map(|m| m.borrow().type_name())
            .unwrap_or_else(|| "None".into());
        eprintln!("IS-ENTER obj={} class={} class_meta={}", o_t, c_t, c_meta);
    }
    if args.len() != 2 {
        return Err(PyError::type_error(
            "isinstance() takes exactly 2 arguments",
        ));
    }
    // `isinstance(x, int | str)` — a PEP 604 union used as the second
    // argument checks membership against ANY of its parts, same as the
    // tuple-of-types form just below (real CPython treats `X | Y` and
    // `(X, Y)` identically here). Checked before borrowing `args[1]` as
    // `class` below since `union_args` does its own borrow internally.
    if let Some(members) = crate::modules::union_args(&args[1]) {
        let _guard = IsinstanceRecursionGuard::enter()?;
        for t in &members {
            // A union member can be the literal `None` singleton (`int |
            // None`, matching real CPython's own PEP 604 syntax) rather than
            // `NoneType` itself — `isinstance(x, None)` isn't meaningful
            // (`None` isn't a class), so check against `type(None)` instead.
            let t = if matches!(&*t.borrow(), PyObject::None) {
                builtin_type_of(&[py_none()])?
            } else {
                t.clone()
            };
            let check_args = vec![args[0].clone(), t];
            if builtin_isinstance(&check_args)?.truthy() {
                return Ok(py_bool(true));
            }
        }
        return Ok(py_bool(false));
    }
    let obj = args[0].borrow();
    let class = args[1].borrow();
    // A class with a custom `__instancecheck__` (collections.abc ABCs like
    // Hashable/Iterable/Sized) delegates the check to it.
    if let PyObject::Type { dict, .. } = &*class {
        if let Some(ic) = dict.get_str("__instancecheck__") {
            if !matches!(&*ic.borrow(), PyObject::None) {
                let result = call_bound_method(ic.clone(), args[1].clone(), vec![args[0].clone()])?;
                return Ok(result);
            }
        }
    }
    // Fallback: `__instancecheck__` defined on the class's METACLASS (the
    // standard CPython placement). The dict-level lookup above only fires
    // for the non-standard in-class placement; dynamically built protocol
    // /ABC classes (`_ProtocolMeta('X', (object,), ns)`) carry the hook on
    // their metaclass instead, and — because dynamic class creation with a
    // custom metaclass currently drops the provided namespace into the
    // new Type's dict — the metaclass MRO is the ONLY place the hook is
    // reachable from for them.
    if let Some(meta) = crate::object::metatype_of(&args[1]) {
        if std::env::var("RPY_TRACE_IS").is_ok() {
            eprintln!("IS-TRACE fallback: class={} meta={}",
                      class.type_name(), meta.borrow().type_name());
        }
        if !meta.is(&args[1]) {
            if let Some(f) = crate::object::lookup_dunder_via_mro(&meta, "__instancecheck__") {
                let result =
                    call_bound_method(f, args[1].clone(), vec![args[0].clone()])?;
                return Ok(result);
            }
        }
    }
    // Handle tuple of types: isinstance(x, (type1, type2, ...))
    if let PyObject::Tuple(types) = &*class {
        let _guard = IsinstanceRecursionGuard::enter()?;
        for t in types {
            let check_args = vec![args[0].clone(), t.clone()];
            if builtin_isinstance(&check_args)?.truthy() {
                return Ok(py_bool(true));
            }
        }
        return Ok(py_bool(false));
    }
    match (&*obj, &*class) {
        (
            PyObject::Type { .. },
            PyObject::Type {
                name: class_name, ..
            },
        ) => {
            // isinstance(SomeClass, X): every class is an instance of its
            // metaclass (a custom one if it was built via `metaclass=`,
            // otherwise plain `type` — e.g. `isinstance(Foo, type)` is
            // True for any ordinary class `Foo`). Walk the metaclass's own
            // mro for `X`, exactly like the Instance case above walks the
            // *class's* mro for ordinary instance checks. Deliberately
            // avoids fetching the canonical `type`/`object` singletons via
            // `with_vm_mut` for the no-custom-metaclass fallback below —
            // `isinstance()` runs deep inside live call chains constantly,
            // and grabbing a second aliasing `&mut VirtualMachine` there
            // reliably segfaulted in testing; a plain name comparison
            // (matching how BuiltinFunction-represented native bases are
            // already recognized elsewhere in this function) avoids it.
            if let Some(mt) = metatype_of(&args[0]) {
                if mt.is(&args[1]) {
                    return Ok(py_bool(true));
                }
                if let PyObject::Type { mro, .. } = &*mt.borrow() {
                    for c in mro {
                        if c.is(&args[1]) {
                            return Ok(py_bool(true));
                        }
                    }
                }
                return Ok(py_bool(false));
            }
            Ok(py_bool(class_name == "type" || class_name == "object"))
        }
        (PyObject::Instance { typ, .. }, PyObject::Type { .. }) => {
            // isinstance(obj, Cls): Cls must appear in obj's OWN type's mro
            // — this used to walk `Cls`'s mro checking for `obj`'s exact
            // class name instead (backwards: comparing the wrong object's
            // ancestry, and by name instead of identity), so
            // `isinstance(subclass_instance, ParentClass)` was always False
            // unless the instance's class name happened to literally match
            // one of Cls's own ancestors' names. Confirmed via a minimal,
            // Django-free repro: `isinstance(CharField(), Field)` (a plain
            // one-level subclass check) returning False — this broke every
            // `isinstance(other, Field)`-style guard in real code (Django's
            // `Field.__lt__`/`__eq__`, used by `@total_ordering` and
            // `bisect`-based field ordering during model construction).
            // Direct identity check FIRST, independent of `mro` — a
            // properly-built class (via `default_build_class`) always has
            // itself as `mro[0]`, so this was previously redundant with
            // the mro walk below and easy to miss. But several ad-hoc
            // native `PyObject::Type`s built directly in Rust (this
            // session's own `Fraction`, `namedtuple`-generated classes,
            // `HTTPConnection`, ...) set `mro: vec![]` (empty) since they
            // aren't constructed through the real class-creation
            // machinery — for those, `isinstance(instance, ExactOwnType)`
            // was ALWAYS `False`, even for the most basic possible check,
            // since the type never appeared in its own (empty) mro at
            // all. Confirmed via `isinstance(Fraction(1,2), Fraction)` and
            // `isinstance(a_namedtuple_instance, ThatNamedtupleType)`,
            // both `False` before this fix.
            if typ.is(&args[1]) {
                return Ok(py_bool(true));
            }
            let typ_ref = typ.borrow();
            if let PyObject::Type { mro, .. } = &*typ_ref {
                for c in mro {
                    if c.is(&args[1]) {
                        return Ok(py_bool(true));
                    }
                }
            }
            drop(typ_ref);
            // `ABCMeta.register(subclass)`-style virtual subclass checks
            // (see `.register`'s own doc comment, `get_attribute`'s
            // `PyObject::Type` arm) — `class` (the ABC) records registered
            // classes in its own `_abc_registry` frozenset (read via
            // `own_abc_registry`, NOT `get_attribute` — see its own doc
            // comment for why inherited/MRO-walked registries are wrong
            // here); `obj`'s class (or any of ITS ancestors, for a real
            // subclass of a registered virtual-subclass root) counts too.
            if abc_registry_matches_in_subtree(&args[1], &|registered| {
                typ.is(registered)
                    || matches!(&*typ.borrow(), PyObject::Type { mro, .. } if mro.iter().any(|c| c.is(registered)))
            }) {
                return Ok(py_bool(true));
            }
            Ok(py_bool(false))
        }
        (PyObject::Instance { typ, .. }, _) => {
            let class_name = match &*class {
                PyObject::BuiltinFunction { name, .. } => name.clone(),
                PyObject::Str(s) => s.to_string(),
                _ => class.str(),
            };
            // `class Foo(list): ...` — Foo transparently subclasses a
            // native builtin, so isinstance(foo, list) must also be true.
            if let Some(native_kind) = native_base_of_type(typ) {
                if native_kind == class_name {
                    return Ok(py_bool(true));
                }
            }
            // `class MyError(Exception): ...` — MyError's instances must
            // also be `isinstance(x, Exception)` (and `AttributeError`,
            // `BaseException`, etc, for whatever it really derives from).
            // Builtin exception "classes" are `PyObject::BuiltinFunction`s
            // that never appear in a custom class's own `mro` (only real
            // `PyObject::Type` bases do), so the mro-walk arm just above
            // can't see this relationship at all — without this, NO custom
            // exception subclass was ever recognized as an instance of any
            // of its real (builtin) ancestors, which also broke `except
            // Exception:`/`except SomeBuiltinBase:` for literally any
            // user-defined exception class (CHECK_EXC_MATCH, `vm.rs`, and
            // `builtin_issubclass` below share this exact same gap and fix).
            if let PyObject::BuiltinFunction { .. } = &*class {
                if let Some(base_name) = find_exception_base_name(typ) {
                    if crate::vm::is_exception_subclass(&base_name, &class_name) {
                        return Ok(py_bool(true));
                    }
                }
            }
            Ok(py_bool(
                typ.borrow().type_name() == class_name || class_name == "object",
            ))
        }
        // `isinstance(TypeError, type)` (and similar for any builtin
        // exception "class") — real CPython: every exception class's
        // metaclass is `type`, so this must be True. Since builtin
        // exception classes are represented as plain `PyObject::BuiltinFunction`s
        // here (not `PyObject::Type`s), the generic name-based hierarchy
        // check in the catch-all arm below instead compared this
        // function's own native `type_name()` ("builtin_function_or_method")
        // against "type" in the EXCEPTION hierarchy table (nonsensical —
        // that table is for exception ANCESTRY, not metaclass checks) and
        // always returned False. Real trigger: `unittest`'s own
        // `case.py`'s `_is_subtype`, `isinstance(expected, type) and
        // issubclass(expected, basetype)` — used by `assertRaises(SomeErr)`
        // to validate its argument — always failed, making
        // `self.assertRaises(TypeError)` (or ANY builtin exception class)
        // raise `TypeError: assertRaises() arg 1 must be an exception type
        // or tuple of exception types` instead of actually working.
        (PyObject::BuiltinFunction { name, .. }, PyObject::Type { name: cname, .. })
            if is_builtin_exception_class_name(name)
                || is_pseudo_type_builtin_function_name(name) =>
        {
            Ok(py_bool(cname == "type" || cname == "object"))
        }
        _ => {
            let obj_type = args[0].borrow().type_name();
            let class_name = match &*class {
                PyObject::BuiltinFunction { name, .. } => name.clone(),
                PyObject::Str(s) => s.to_string(),
                PyObject::Type { name, .. } => name.clone(),
                _ => class.str(),
            };
            // Direct type name match for built-in types (int, str, list, etc.)
            if obj_type == class_name {
                return Ok(py_bool(true));
            }
            // `bool` is real CPython's one primitive with an actual
            // inheritance relationship to another primitive
            // (`bool.__bases__ == (int,)`) — `isinstance(True, int)` and
            // `isinstance(True, object)` must both be `True`. A `bool`
            // value here is always the PRIMITIVE `PyObjectRef::SmallBool`/
            // `PyObject::Bool` shape (never a `PyObject::Instance`, since
            // `bool` can't be subclassed — see `default_build_class`'s
            // explicit block for that), so this can't be expressed via the
            // generic mro-walk arms above (which all key off an `Instance`'s
            // `typ`) — a narrow, direct name check is the simplest correct
            // fix, mirroring this function's existing style for other
            // primitive/name-based special cases.
            if obj_type == "bool" && (class_name == "int" || class_name == "object") {
                return Ok(py_bool(true));
            }
            // See `builtin_isinstance`'s other `_abc_registry` fallback —
            // this one covers a PRIMITIVE (inline `SmallInt`/`SmallStr`/...
            // or boxed `Int`/`Str`/...) checked against an ABC that
            // registered the matching builtin type name (real trigger:
            // `numbers.py`'s own `Integral.register(int)`/`Real.register
            // (float)`/`Complex.register(complex)` — needed for
            // `isinstance(5, numbers.Integral)` to work at all).
            if let PyObject::Type { .. } = &*class {
                if abc_registry_matches_in_subtree(&args[1], &|registered| {
                    let registered_name = match &*registered.borrow() {
                        PyObject::BuiltinFunction { name, .. } => name.clone(),
                        PyObject::Type { name, .. } => name.clone(),
                        _ => return false,
                    };
                    registered_name == obj_type
                }) {
                    return Ok(py_bool(true));
                }
            }
            // Exception hierarchy — but only for objects that can actually
            // BE exceptions: real `PyObject::Exception` instances (builtin
            // or user-defined subclass), or builtin exception CLASS names.
            // `is_exception_subclass` maps unknown type names to `Exception`
            // by default (so user-defined `class MyError(Exception)` bodies
            // resolve correctly), which wrongly made ANY primitive value —
            // `isinstance('x', BaseException)`, `isinstance([], ValueError)`
            // — resolve through the exception table and return True. Real
            // trigger: `Lib/test/support/os_helper.py`'s `FakePath.__fspath__`
            // guards on `isinstance(self.path, BaseException)`, and a str
            // path incorrectly took the exception-raising branch, so every
            // `os.path.*`/`open()` call handed a `FakePath` failed.
            if matches!(&*obj, PyObject::Exception { .. })
                || is_builtin_exception_class_name(&obj_type)
            {
                return Ok(py_bool(crate::vm::is_exception_subclass(
                    &obj_type,
                    &class_name,
                )));
            }
            Ok(py_bool(false))
        }
    }
}


/// Real `open()`/`os.*` path arguments accept `str`, `bytes`, or any
/// `os.PathLike` — but `PyObjectRef::str()` on a `PyObject::Bytes` value
/// deliberately returns Python's own `repr()`-style `"b'...'"` form (matching
/// real CPython's `str(b"x")` semantics — NOT a bug on its own), which is
/// exactly the WRONG thing to feed to a filesystem call. Real trigger:
/// CPython's own `dbm/dumb.py` (via `os.fsencode`), which builds its
/// `.dat`/`.dir`/`.bak` filenames as `bytes` and passes them straight to
/// `io.open()` — `open(b"/tmp/x.dat", "w")` tried to open a file literally
/// named `b'/tmp/x.dat'` (quotes, `b` prefix and all), which obviously
/// doesn't exist, raising a confusing `OSError` instead of writing to the
/// intended path.
pub(crate) fn path_arg_to_string(obj: &PyObjectRef) -> String {
    // PEP 519 `os.PathLike` protocol: any object with a `__fspath__()`
    // method (real code: `pathlib.Path`, and plenty of test-only wrappers
    // like `Lib/test/support/os_helper.py`'s `FakePath`) must have THAT
    // called to get the real path, instead of falling through to
    // `.str()` — which, for a plain custom class instance with no
    // `__str__` override, produces its REPR (`<FakePath '/tmp/xxx'>`,
    // literally including the wrapper's own class name and quoting) rather
    // than the real path string, so every `open()`/`os.*` call given such
    // a wrapped path silently tried to open a nonexistent file named after
    // its own repr instead. Confirmed via `test_dbm.py::test_whichdb`,
    // which explicitly exercises `FakePath`-wrapped paths.
    let f = {
        let o = obj.borrow();
        if let PyObject::Instance { typ, .. } = &*o {
            lookup_dunder_via_mro(typ, "__fspath__")
        } else {
            None
        }
    };
    if let Some(f) = f {
        if let Ok(result) = call_bound_method(f, obj.clone(), vec![]) {
            return path_arg_to_string(&result);
        }
    }
    if let PyObject::Bytes(b) = &*obj.borrow() {
        String::from_utf8_lossy(b).into_owned()
    } else {
        obj.str()
    }
}


pub fn builtin_open(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error(
            "open() missing required argument 'file'",
        ));
    }
    // A keyword call (e.g. the extremely common `open(path, encoding="utf-8")`,
    // with NO explicit `mode`) reaches every plain `BuiltinFunction` with its
    // keywords packed into a dict APPENDED as the last positional arg (see
    // `vm.rs`'s `call_function`) — this was read directly as `args[1]` (the
    // "mode" position) whenever ANY keyword was passed, regardless of
    // whether `mode` itself was one of them. `dict.str()` on that packed
    // kwargs dict produced something like `"{'encoding': 'utf-8'}"`, which
    // contains none of 'r'/'w'/'a'/'+' — so NO read/write/append flag ever
    // got set, and the file open failed with a raw, confusing `OSError:
    // must specify at least one of read, write, or append access` instead
    // of just opening for reading (the real, correct default). Confirmed
    // via `test_baseexception.py::test_inheritance`'s own `open(path,
    // encoding="utf-8")` call. Now separates a trailing kwargs dict (if
    // present — `open()`'s real parameters are never legitimately a dict
    // themselves, so this is unambiguous) and reads `mode` from either the
    // second positional arg or the `mode` keyword, defaulting to `"r"`.
    let (pos_args, kwargs) = match args.last() {
        Some(last) if matches!(&*last.borrow(), PyObject::Dict(_)) => {
            (&args[..args.len() - 1], Some(last))
        }
        _ => (args, None),
    };
    let filename = path_arg_to_string(&pos_args[0]);
    let mode = if pos_args.len() > 1 {
        pos_args[1].str()
    } else if let Some(kw) = kwargs {
        if let PyObject::Dict(d) = &*kw.borrow() {
            d.get(&py_str("mode"))
                .ok()
                .flatten()
                .map(|v| v.str())
                .unwrap_or_else(|| "r".to_string())
        } else {
            "r".to_string()
        }
    } else {
        "r".to_string()
    };
    // A trailing `+` ("r+"/"w+"/"a+", real CPython's "and updating" suffix)
    // means the file is opened for BOTH reading and writing — was
    // completely ignored here, so "rb+" (read-write, don't truncate, don't
    // create — the exact mode `dbm/dumb.py` uses to append new values to
    // its own data file) only ever opened for reading, and a subsequent
    // `f.write(...)` failed with a raw OS-level "Bad file descriptor"
    // instead of writing.
    let has_plus = mode.contains('+');
    // Mode `'x'` = exclusive create (real CPython's third create flag,
    // alongside `'w'` create+truncate and `'a'` create+append): creates the
    // file but FAILS with `FileExistsError` if it already exists. Was
    // completely unrecognized — `open(path, 'xb')` (the very common "write
    // this file only if I'm not clobbering something" idiom, used all over
    // CPython's own test suite) set NO read/write flag at all, failing with
    // a raw `OSError: must specify at least one of read, write, or append
    // access`. It implies write, exactly like `'w'`.
    let has_x = mode.contains('x');
    let mut opts = std::fs::File::options();
    opts.read(mode.contains('r') || has_plus)
        .write(mode.contains('w') || mode.contains('a') || has_plus || has_x)
        .append(mode.contains('a'))
        .create(mode.contains('w') || mode.contains('a') || has_x)
        .truncate(mode.contains('w'));
    if has_x {
        opts.create_new(true);
    }
    let file = opts
        .open(&filename)
        .map_err(|e| PyError::os_error_from_io(&e))?;
    let binary = mode.contains('b');
    Ok(PyObjectRef::new(PyObject::File {
        file: std::rc::Rc::new(std::cell::RefCell::new(file)),
        name: filename,
        binary,
        pending: std::rc::Rc::new(std::cell::RefCell::new(Vec::new())),
        closed: false,
    }))
}


// Python-semantics modulo for `BigInt` (result takes the SIGN OF THE
// DIVISOR, unlike Rust's `%`, which takes the sign of the dividend) — needed
// by `builtin_pow`'s 3-arg form below, whose test coverage explicitly checks
// negative moduli (`test_pow.py::test_negative_exponent` sweeps `m` from
// -50 to 49).


pub fn builtin_issubclass(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 2 {
        return Err(PyError::type_error(
            "issubclass() takes exactly 2 arguments",
        ));
    }
    // `issubclass(cls, int | str)` — same PEP 604 union-membership check as
    // `builtin_isinstance`'s matching case just above.
    if let Some(members) = crate::modules::union_args(&args[1]) {
        let _guard = IsinstanceRecursionGuard::enter()?;
        for t in &members {
            let t = if matches!(&*t.borrow(), PyObject::None) {
                builtin_type_of(&[py_none()])?
            } else {
                t.clone()
            };
            let check_args = vec![args[0].clone(), t];
            if builtin_issubclass(&check_args)?.truthy() {
                return Ok(py_bool(true));
            }
        }
        return Ok(py_bool(false));
    }
    // Custom __subclasscheck__ (e.g. os.PathLike virtual subclass)
    {
        let maybe_sc = {
            let base_b = args[1].borrow();
            if let PyObject::Type { dict, .. } = &*base_b {
                dict.get_str("__subclasscheck__").cloned()
            } else {
                None
            }
        };
        if let Some(sc) = maybe_sc {
            if !matches!(&*sc.borrow(), PyObject::None) {
                let result = call_bound_method(sc, args[1].clone(), vec![args[0].clone()])?;
                return Ok(result);
            }
        }
        if let Some(meta) = crate::object::metatype_of(&args[1]) {
            if !meta.is(&args[1]) {
                if let Some(f) = crate::object::lookup_dunder_via_mro(&meta, "__subclasscheck__") {
                    let result = call_bound_method(f, args[1].clone(), vec![args[0].clone()])?;
                    return Ok(result);
                }
            }
        }
    }
    // A genuinely bogus second argument (not a class, tuple of classes, or
    // union, and with no custom `__subclasscheck__`/metaclass override —
    // already ruled out just above) must raise `TypeError`, matching real
    // CPython, rather than silently falling through to the name-based
    // matching below and returning `False`. Real trigger: `test_abc.py`'s
    // `test_issubclass_bad_arguments` (bpo-34441) — `issubclass(int, S)`
    // where `S.__subclasses__` is overridden to return a list containing a
    // non-class (`42`); `_py_abc.py`'s own `__subclasscheck__` then calls
    // `issubclass(subclass, 42)` while walking that list, which must raise
    // `TypeError` for the whole check to propagate correctly (previously it
    // returned `False`, so `_py_abc.py` just kept walking instead of
    // erroring, and `assertRaises(TypeError, ...)` failed).
    {
        let base_b = args[1].borrow();
        let is_bogus = !matches!(
            &*base_b,
            PyObject::Type { .. }
                | PyObject::BuiltinFunction { .. }
                | PyObject::Tuple(_)
                // A bare `Str` is accepted here too — this codebase's own
                // internal fallback (see the `(PyObject::Str(cls_name), _)`
                // arm below, `WITH_EXIT`'s module-exception-by-name lookup)
                // can legitimately pass a plain string as EITHER argument
                // when a module-scoped exception has no real `Type`/
                // `BuiltinFunction` representation reachable from the
                // current frame's builtins.
                | PyObject::Str(_)
        );
        if is_bogus {
            return Err(PyError::type_error(
                "issubclass() arg 2 must be a class, a tuple of classes, or a union",
            ));
        }
    }
    // Handle tuple of types: issubclass(cls, (type1, type2, ...))
    let base = args[1].borrow();
    if let PyObject::Tuple(types) = &*base {
        let _guard = IsinstanceRecursionGuard::enter()?;
        for t in types {
            let check_args = vec![args[0].clone(), t.clone()];
            if builtin_issubclass(&check_args)?.truthy() {
                return Ok(py_bool(true));
            }
        }
        return Ok(py_bool(false));
    }
    let cls = args[0].borrow();
    drop(base);
    let base = args[1].borrow();
    match (&*cls, &*base) {
        (PyObject::Type { mro: cls_mro, .. }, PyObject::Type { .. }) => {
            // Direct identity check first — same fix, same reason, as
            // `builtin_isinstance`'s matching arm just above: several
            // ad-hoc native `PyObject::Type`s (`Fraction`, `namedtuple`-
            // generated classes, ...) have an empty `mro`, so
            // `issubclass(X, X)` was `False` even for the most basic
            // possible check.
            if args[0].is(&args[1]) {
                return Ok(py_bool(true));
            }
            // Identity, NOT name — two unrelated classes that merely happen
            // to share a `__name__` (e.g. a test helper that does `C =
            // type('C', (Base,), {...})` twice, once with each shape, an
            // extremely common real-world pattern — "C"/"Foo"/"Base" are
            // popular throwaway names) are NOT subclasses of each other.
            // This used to compare `c.borrow().type_name() == base_tn`
            // (a bare string match), so `issubclass(C2, C1)` was `True` for
            // ANY two same-named classes regardless of actual ancestry —
            // real trigger: real `Lib/_collections_abc.py`'s `ABCMeta.
            // __subclasscheck__`'s own `for scls in cls.__subclasses__():
            // if issubclass(subclass, scls): ...` fallback, walking EVERY
            // known subclass of an ABC and issubclass-testing each — the
            // moment ANY real subclass of an ABC existed (from an earlier,
            // unrelated test) with the same popular name as a later,
            // deliberately-UNRELATED throwaway class, that later class was
            // wrongly reported as a subclass too
            // (`test_collections.py`'s `TestOneTrickPonyABCs.
            // validate_isinstance`, which does exactly this: two separate
            // `type('C', ...)` calls, one meant to match an ABC and one
            // deliberately NOT supposed to).
            for c in cls_mro {
                if c.is(&args[1]) {
                    return Ok(py_bool(true));
                }
            }
            // See `builtin_isinstance`'s matching fallback for
            // `ABCMeta.register`-style virtual subclass checks.
            if abc_registry_matches_in_subtree(&args[1], &|registered| {
                cls_mro.iter().any(|c| c.is(registered))
            }) {
                return Ok(py_bool(true));
            }
            Ok(py_bool(false))
        }
        (PyObject::Type { mro: cls_mro, .. }, _) => {
            // Non-Type second argument: compare by name. `.str()` on a
            // BuiltinFunction (how builtin exception "classes" like
            // `Exception` are represented) returns its full repr
            // (`<built-in function Exception>`), not the bare name — must
            // extract that explicitly or every name comparison below
            // (including the exception-ancestry fix just past the mro
            // walk) silently never matches.
            let base_name = match &*base {
                PyObject::BuiltinFunction { name, .. } => name.clone(),
                _ => base.str(),
            };
            if base_name == "object" {
                return Ok(py_bool(true));
            }
            for c in cls_mro {
                if c.borrow().type_name() == base_name {
                    return Ok(py_bool(true));
                }
            }
            // `issubclass(MyError, Exception)` where MyError is a real
            // user-defined `class MyError(Exception): ...` — Exception (a
            // `BuiltinFunction`, not a `Type`) never appears in MyError's own
            // `mro`, so the walk above can't see this. Same gap/fix as
            // isinstance()'s Instance/BuiltinFunction arm just above.
            if matches!(&*base, PyObject::BuiltinFunction { .. }) {
                if let Some(base_exc_name) = find_exception_base_name(&args[0]) {
                    if crate::vm::is_exception_subclass(&base_exc_name, &base_name) {
                        return Ok(py_bool(true));
                    }
                }
            }
            Ok(py_bool(false))
        }
        // Built-in exception "classes" (e.g. KeyError, ValueError) are
        // represented as BuiltinFunction constructors rather than Type
        // objects — resolve ancestry by name via the same table `except`
        // and isinstance() use, instead of only accepting real Type values.
        // A BuiltinFunction that is NOT a recognized exception class is not
        // a class at all (real CPython: issubclass(abs, X) raises TypeError)
        // — without this gate, `is_exception_subclass`'s catch-all mapped
        // ANY BuiltinFunction name (abs, print, ...) to Exception, so
        // `issubclass(abs, BaseException)` returned True.
        (PyObject::BuiltinFunction { name: cls_name, .. }, _) => {
            if !is_builtin_exception_class_name(cls_name)
                && !is_pseudo_type_builtin_function_name(cls_name)
            {
                return Err(PyError::type_error("issubclass() arg 1 must be a class"));
            }
            let base_name = match &*base {
                PyObject::BuiltinFunction { name, .. } => name.clone(),
                PyObject::Str(s) => s.to_string(),
                PyObject::Type { name, .. } => name.clone(),
                _ => base.str(),
            };
            // Everything (including every exception "class") is a subclass of
            // `object` — `issubclass(Exception, object)` must be True
            // (test_baseexception::test_builtins_new_style).
            if base_name == "object" {
                return Ok(py_bool(true));
            }
            // `is_exception_subclass`'s own catch-all treats any name it
            // doesn't recognize as a direct `Exception` subclass (a
            // separate, legitimate need — see its own comment — for
            // module-scoped exceptions reached only by name via
            // `WITH_EXIT`'s lookup). `range`/`memoryview`/`property`/etc.
            // are real (non-exception) pseudo-types allowed past the guard
            // above for the `object`/abc-registry checks only; routing
            // them into that catch-all wrongly reported
            // `issubclass(range, BaseException)` as `True`
            // (test_baseexception.py's `test_inheritance`, which walks
            // every value in `builtins.__dict__` checking exactly this).
            if is_builtin_exception_class_name(cls_name)
                && crate::vm::is_exception_subclass(cls_name, &base_name)
            {
                return Ok(py_bool(true));
            }
            // See `builtin_isinstance`'s `_abc_registry` fallback — this
            // covers `issubclass(int, numbers.Integral)`-style checks
            // where the SUBCLASS side (`int`/`float`/`complex`/...) is
            // itself a `BuiltinFunction`, not a `Type`. Real trigger:
            // `numbers.py`'s own `Integral.register(int)`/`Real.register
            // (float)`/`Complex.register(complex)`.
            if let PyObject::Type { .. } = &*base {
                if abc_registry_matches_in_subtree(&args[1], &|registered| {
                    let registered_name = match &*registered.borrow() {
                        PyObject::BuiltinFunction { name, .. } => name.clone(),
                        PyObject::Type { name, .. } => name.clone(),
                        _ => return false,
                    };
                    &registered_name == cls_name
                }) {
                    return Ok(py_bool(true));
                }
            }
            Ok(py_bool(false))
        }
        // `Opcode::WITH_EXIT` (`vm.rs`) resolves a raised `PyObject::Exception`'s
        // `exc_type` argument to `__exit__` by looking its `typ` name up in the
        // CURRENT frame's builtins — which only ever holds the ~70 core
        // exception names (`add_exc_type!` in `create_builtins`), never a
        // module-scoped custom exception like `struct.error`/`pickle.PickleError`
        // (those live only in their own module's dict, not global builtins).
        // When that lookup misses, it falls back to a bare `PyObject::Str`
        // holding just the name, as a last-resort placeholder — so
        // `issubclass(exc_type, struct.error)` inside `unittest`'s own
        // `_AssertRaisesBaseContext.__exit__` reached here with a plain string
        // for `cls`. This codebase already treats built-in/module exception
        // "classes" as interchangeable by name everywhere else (the
        // `BuiltinFunction`/`Type`/`Str` arms just above), so extending that
        // same name-based comparison to a bare string `cls` here is consistent
        // for that INTERNAL fallback specifically — but a real user calling
        // `issubclass("hello", BaseException)` must still get real Python's
        // `TypeError: issubclass() arg 1 must be a class` (previously this
        // arm accepted ANY string unconditionally, so a plain string that
        // merely happened to arrive here — confirmed via CPython's own
        // `test_baseexception.py`, whose `test_inheritance`/`test_catch_string`
        // pass arbitrary strings including plain object values from
        // `builtins.__dict__` — silently returned `False`/`True` by name
        // comparison instead of raising). Gated on `cls_name` actually being
        // one of the recognized builtin/module exception names — every
        // legitimate internal fallback string is drawn from exactly that set
        // (`add_exc_type!`'s own names, or a module exception registered via
        // `is_builtin_exception_class_name`), so an unrecognized string can
        // only be genuine user input, not this internal fallback.
        (PyObject::Str(cls_name), _) if is_builtin_exception_class_name(cls_name) => {
            let base_name = match &*base {
                PyObject::BuiltinFunction { name, .. } => name.clone(),
                PyObject::Str(s) => s.to_string(),
                PyObject::Type { name, .. } => name.clone(),
                _ => base.str(),
            };
            Ok(py_bool(crate::vm::is_exception_subclass(
                cls_name, &base_name,
            )))
        }
        _ => {
            if std::env::var("RPY_DEBUG_ISSUBCLASS").is_ok() {
                eprintln!(
                    "issubclass() FAIL: arg0={:?}/{} arg1={:?}/{}",
                    cls.type_name(),
                    cls.repr(),
                    base.type_name(),
                    base.repr()
                );
            }
            Err(PyError::type_error("issubclass() arg 1 must be a class"))
        }
    }
}


pub fn builtin_help(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        println!("Welcome to RustPython 0.1.0!");
        println!();
        println!("Available built-in functions:");
        println!("  abs()  all()  any()  ascii()  bin()  bool()  breakpoint()");
        println!("  bytearray()  bytes()  callable()  chr()  compile()  delattr()");
        println!("  dict()  dir()  divmod()  enumerate()  eval()  exec()  exit()");
        println!("  filter()  float()  format()  frozenset()  getattr()  globals()");
        println!("  hasattr()  hash()  help()  hex()  id()  input()  int()");
        println!("  isinstance()  issubclass()  iter()  len()  list()  locals()");
        println!("  map()  max()  memoryview()  min()  next()  object()  oct()");
        println!("  open()  ord()  pow()  print()  property()  range()  repr()");
        println!("  reversed()  round()  set()  setattr()  slice()  sorted()");
        println!("  staticmethod()  str()  sum()  super()  tuple()  type()  vars()");
        println!("  zip()");
        println!();
        println!("Available error types:");
        println!("  BaseException  Exception  TypeError  ValueError  ZeroDivisionError");
        println!("  NameError  AttributeError  IndexError  KeyError  RuntimeError");
        println!("  StopIteration  AssertionError  OSError  ImportError  LookupError");
        println!("  ArithmeticError  OverflowError  NotImplementedError  RecursionError");
        println!("  KeyboardInterrupt  SystemExit  ModuleNotFoundError  FileNotFoundError");
        println!("  PermissionError  UnicodeDecodeError  UnicodeEncodeError");
        println!();
        println!("Type help(object) for information about a specific object.");
    } else {
        let obj = args[0].borrow();
        match &*obj {
            PyObject::Type { name, dict, .. } => {
                println!("Help on class {}:", name);
                if let Some(doc) = dict.get_str("__doc__") {
                    println!("  {}", doc.str());
                }
                println!();
                println!("Methods:");
                for (key, val) in dict.iter() {
                    if matches!(
                        &*val.borrow(),
                        PyObject::Function(_) | PyObject::BuiltinFunction { .. }
                    ) {
                        println!("  {}()", interner::lookup_str(*key));
                    }
                }
            }
            PyObject::Function(ref f) => {
                let name = &f.code.name;
                let dict = &f.dict;
                println!("Help on function {}:", name);
                if let Some(doc) = dict.get("__doc__") {
                    println!("  {}", doc.str());
                }
            }
            PyObject::BuiltinFunction { name, .. } => {
                println!("Help on built-in function {}:", name);
            }
            _ => {
                println!("Help on {}:", obj.type_name());
                println!("  Type: {}", obj.type_name());
            }
        }
    }
    Ok(py_none())
}
