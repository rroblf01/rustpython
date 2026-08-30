// Extracted from src/object/attrs/mod.rs — Type/Instance/Property/StaticMethod/ClassMethod attribute dispatch
use crate::object::*;
use super::*;
use crate::interner;
use std::rc::Rc;

pub(crate) fn get(o: &PyObject, name: &str) -> PyResult<PyObjectRef> {
    match o {
            PyObject::Type {
                dict,
                mro,
                bases,
                name: type_name,
            } => {
                if name == "__dict__" {
                    // Return type's dict as a PyDict — NATIVE_BASE_MARKER is
                    // an internal bookkeeping entry (see native_base_of_type)
                    // and must not leak into user-visible introspection.
                    let mut pd = PyDict::new();
                    for (k, v) in dict.iter() {
                        let k_str = interner::lookup_str(*k);
                        if k_str == NATIVE_BASE_MARKER
                            || k_str == METATYPE_KEY
                            || k_str == NATIVE_VALUE_CTOR_KEY
                        {
                            continue;
                        }
                        let _ = pd.set(py_str(k_str), v.clone());
                    }
                    return Ok(PyObjectRef::new(PyObject::Dict(Box::new(pd))));
                }
                if name == "__mro__" {
                    return Ok(PyObjectRef::new(PyObject::Tuple(mro.clone())));
                }
                if name == "__bases__" {
                    return Ok(PyObjectRef::new(PyObject::Tuple(bases.clone())));
                }
                if name == "__base__" {
                    // CPython: `__base__` is the first of `__bases__` (the
                    // "best base" in multiple-inheritance cases, but for
                    // every class this codebase constructs there's at most
                    // one real native/exception base, so first-is-fine);
                    // `object` itself falls back to `None`. Was missing
                    // entirely — needed so e.g. `socket.gaierror.__base__
                    // is OSError` can be checked at all.
                    return Ok(bases
                        .first()
                        .cloned()
                        .or_else(|| crate::object::get_primitive_type("object"))
                        .unwrap_or_else(py_none));
                }
                if name == "__name__" {
                    return Ok(py_str(type_name));
                }
                if name == "__qualname__" {
                    return Ok(py_str(type_name));
                }
                if name == "__annotations__" {
                    if let Some(v) = dict.get_str("__annotations__").cloned() {
                        return Ok(v);
                    }
                    return Ok(crate::object::py_dict());
                }
                // `__module__` — real user-defined classes already have this
                // seeded into their own dict at class-creation time (the
                // class body's implicit `__module__ = __name__` statement),
                // so this fallback is only ever reached for BUILTIN/native
                // ad-hoc types (`int`, `str`, `types.UnionType`, ...), which
                // never went through that seeding. Defaults to `"builtins"`
                // (correct for the real native types; a reasonable filler
                // for ad-hoc "instance-shaped" native types like `Path`/
                // `SimpleNamespace`/`UnionType`, which have no real module
                // of their own to report) — found via CPython's own
                // `test_types.py`'s `check_disallow_instantiation` helper,
                // which unconditionally reads `tp.__module__` on ANY type.
                if name == "__module__" && !dict.contains_key_str("__module__") {
                    // `array`'s instances live in the `array` module —
                    // reprlib's dispatch keys on `type(x).__module__`
                    // (test_reprlib::test_container).
                    if type_name == "array" {
                        return Ok(py_str("array"));
                    }
                    return Ok(py_str("builtins"));
                }
                // PEP 604 union syntax (`int | str`, `MyClass | None`) — the
                // `|` operator was entirely unsupported on ANY class/builtin
                // type (`TypeError: unsupported operand type(s) for |: ...`)
                // even though it's an extremely common modern idiom in type
                // annotations (`def f(x: int | str)`) and isinstance checks
                // (`isinstance(x, int | None)`), evaluated at RUNTIME
                // whenever the annotation isn't behind `from __future__
                // import annotations`. Gated on the type's own dict NOT
                // already defining `__or__`/`__ror__` (same pattern as
                // `register` just above) so a class that genuinely overrides
                // either keeps its own behavior.
                if (name == "__or__" || name == "__ror__") && !dict.contains_key_str(name) {
                    // A plain `BuiltinFunction`, NOT `BuiltinMethod` — the
                    // latter's `call_bound_method` convention prepends an
                    // extra placeholder `self_obj` ahead of `self`/`other`
                    // (3 args: `[None, self, other]`), which silently
                    // shifted every argument here by one (confirmed via a
                    // direct repro: `int | str` built a union of `[None,
                    // int]` instead of `[int, str]`). `BuiltinFunction`'s own
                    // convention is the plain 2-arg `[self, other]` these
                    // closures actually expect — see `try_dunder_binop`'s own
                    // doc comment for the exact convention split between the
                    // two.
                    return Ok(PyObjectRef::imm(PyObject::BuiltinFunction {
                        name: name.to_string(),
                        func: if name == "__or__" {
                            |args| {
                                if args.len() < 2 {
                                    return Err(PyError::type_error("__or__() missing argument"));
                                }
                                Ok(crate::modules::make_union(vec![
                                    args[0].clone(),
                                    args[1].clone(),
                                ]))
                            }
                        } else {
                            |args| {
                                if args.len() < 2 {
                                    return Err(PyError::type_error("__ror__() missing argument"));
                                }
                                Ok(crate::modules::make_union(vec![
                                    args[1].clone(),
                                    args[0].clone(),
                                ]))
                            }
                        },
                    }));
                }
                // `ABCMeta.register(subclass)` — real CPython's `abc.py`
                // wraps a native `_abc_register` primitive that this
                // project already implements (`modules/core.rs`) but never
                // actually wires up: `class Foo(metaclass=ABCMeta): ...`
                // doesn't go through a real `class ABCMeta(type):` (this
                // project's own `ABCMeta` is a plain `BuiltinFunction`, not
                // a `type` subclass — real per-metaclass method lookup
                // falling back from `SomeClass.register` to `type
                // (SomeClass).register` is a deeper, unimplemented
                // architecture piece), so `SomeClass.register` never
                // resolved to anything at all. Providing `.register` as a
                // generic fallback on EVERY class (not gated on "was this
                // built via ABCMeta") is pragmatic rather than fully
                // correct — but calling `.register()` on a non-ABC class
                // isn't something real code does unintentionally, so
                // there's no real-world downside. Records the virtual
                // subclass in a `_abc_registry` frozenset attribute on the
                // class; `isinstance`/`issubclass` consult it (see
                // `builtin_isinstance`/`builtin_issubclass`). Real trigger:
                // `numbers.Number.register(Decimal)` — needed by real
                // CPython's own (vendored) `_pydecimal.py`.
                if name == "register" && !dict.contains_key_str("register") {
                    return Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "register".to_string(),
                        func: |args| {
                            if args.len() < 2 {
                                return Err(PyError::type_error(
                                    "register() takes exactly one argument",
                                ));
                            }
                            let cls = &args[0];
                            let subclass = args[1].clone();
                            // Read the registry from `cls`'s OWN dict only
                            // — NOT via `get_attribute` (which walks the
                            // MRO). `Real.register(float)` must not see
                            // (and then re-save as ITS OWN registry,
                            // permanently merging the two) whatever
                            // `Complex.register(complex)` already stored,
                            // just because `Real` is a subclass of
                            // `Complex` and doesn't have its own registry
                            // entry yet. Confirmed via `numbers.py`'s own
                            // `Complex.register(complex)`/`Real.register
                            // (float)`/`Integral.register(int)`: without
                            // this, `Integral._abc_registry` ended up
                            // accumulating `{complex, float, int}` (all
                            // three merged in), making `issubclass(complex,
                            // Integral)` wrongly `True`.
                            let existing: Vec<PyObjectRef> =
                                if let PyObject::Type { dict, .. } = &*cls.borrow() {
                                    dict.get_str("_abc_registry")
                                        .and_then(|r| {
                                            if let PyObject::FrozenSet(items) = &*r.borrow() {
                                                Some(items.to_vec())
                                            } else {
                                                None
                                            }
                                        })
                                        .unwrap_or_default()
                                } else {
                                    Vec::new()
                                };
                            if !existing.iter().any(|r| r.is(&subclass)) {
                                let mut set = PySet::new();
                                for item in &existing {
                                    set.add(item.clone())?;
                                }
                                set.add(subclass.clone())?;
                                cls.borrow_mut().set_attribute(
                                    "_abc_registry",
                                    PyObjectRef::imm(PyObject::FrozenSet(set)),
                                )?;
                            }
                            Ok(subclass)
                        },
                        self_obj: py_none(),
                    }));
                }
                if name == "__subclasses__" && !dict.contains_key_str("__subclasses__") {
                    // NOTE: self_obj here is a placeholder — LOAD_ATTR's fast
                    // path always rebinds it to the actual accessed object
                    // (`Foo`, for `Foo.__subclasses__`) before calling, so the
                    // real class must be read back out of args[0] at call time
                    // (matching the `mro` method right below).
                    return Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "__subclasses__".to_string(),
                        func: |args| Ok(py_list(direct_subclasses_of(&args[0]))),
                        self_obj: py_none(),
                    }));
                }
                if name == "mro" && !dict.contains_key_str("mro") {
                    // NOTE: self_obj here is a placeholder — LOAD_ATTR's fast
                    // path always rebinds it to the actual accessed object
                    // (`Foo`, for `Foo.mro`) before calling, so the real mro
                    // must be read back out of args[0] at call time.
                    return Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                        name: "mro".to_string(),
                        func: |args| {
                            if let PyObject::Type { mro, .. } = &*args[0].borrow() {
                                Ok(py_list(mro.clone()))
                            } else {
                                Err(PyError::type_error("mro() requires a type object"))
                            }
                        },
                        self_obj: py_none(),
                    }));
                }
                if std::env::var("RPY_DEBUG_TYPEATTR").is_ok() && name == "strip" {
                    eprintln!(
                        "TYPEATTR type={} name={} dict_has={}",
                        type_name,
                        name,
                        dict.contains_key_str(&name)
                    );
                }
                // Check own dict first
                if let Some(val) = dict.get_str(&name).cloned() {
                    // Unwrap staticmethod descriptor so type access returns the function directly
                    let b = val.borrow();
                    if let PyObject::StaticMethod { func } = &*b {
                        return Ok(func.clone());
                    }
                    drop(b);
                    return Ok(val);
                }
                // Check MRO (skip self)
                for base in mro.iter().skip(1) {
                    if let PyObject::Type {
                        dict: base_dict, ..
                    } = &*base.borrow()
                    {
                        if let Some(val) = base_dict.get_str(&name) {
                            // Unwrap staticmethod descriptor from MRO bases
                            let b = val.borrow();
                            if let PyObject::StaticMethod { func } = &*b {
                                return Ok(func.clone());
                            }
                            drop(b);
                            return Ok(val.clone());
                        }
                    }
                }
                // Fallback: for dict-derived types, provide common dict methods
                if name == "__iter__"
                    || name == "items"
                    || name == "keys"
                    || name == "values"
                    || name == "get"
                {
                    static DICT_METHODS: std::sync::OnceLock<
                        std::collections::HashMap<String, BuiltinFunc>,
                    > = std::sync::OnceLock::new();
                    let methods = DICT_METHODS.get_or_init(|| {
                        let mut m = std::collections::HashMap::new();
                        m.insert("__iter__".to_string(), dict_method_iter as BuiltinFunc);
                        m.insert("items".to_string(), dict_method_items as BuiltinFunc);
                        m.insert("keys".to_string(), dict_method_keys as BuiltinFunc);
                        m.insert("values".to_string(), dict_method_values as BuiltinFunc);
                        m.insert("get".to_string(), dict_method_get as BuiltinFunc);
                        m
                    });
                    if let Some(func) = methods.get(name) {
                        let func = *func;
                        // A plain `BuiltinFunction`, NOT `BuiltinMethod` — this
                        // is reached via `dict.keys` (attribute access on the
                        // TYPE itself, for the unbound-call idiom `dict.keys
                        // (self)` a dict subclass uses to invoke the parent's
                        // real implementation) rather than `some_dict.keys()`
                        // (bound instance access, handled elsewhere). A
                        // `BuiltinMethod`'s calling convention prepends its
                        // OWN `self_obj` ahead of whatever args the caller
                        // passes — with a `py_none()` placeholder here, that
                        // shifted every real argument by one (`dict.keys(d)`
                        // called `dict_method_keys(&[None, d])`, so `args[0]`
                        // was never `d` at all) — confirmed via direct repro
                        // (`dict.keys({'a': 1})` unconditionally failed).
                        // `BuiltinFunction`'s plain pass-through convention is
                        // what an unbound-style call actually needs.
                        return Ok(PyObjectRef::new(PyObject::BuiltinFunction {
                            name: name.to_string(),
                            func,
                        }));
                    }
                }
                if matches!(type_name.as_str(), "set" | "frozenset") {
                    let set_op: Option<BuiltinFunc> = match name {
                        "__sub__" => Some(|args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                            if args.len() < 2 { return Err(PyError::type_error("__sub__ missing args")); }
                            let a = crate::object::convert_to_set(&args[0])?;
                            let b = crate::object::convert_to_set(&args[1])?;
                            let mut res = crate::object::PySet::new();
                            for item in a.to_vec() {
                                if !b.contains(&item).unwrap_or(false) { res.add(item)?; }
                            }
                            Ok(PyObjectRef::new(PyObject::Set(res)))
                        }),
                        "__and__" => Some(|args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                            if args.len() < 2 { return Err(PyError::type_error("__and__ missing args")); }
                            let a = crate::object::convert_to_set(&args[0])?;
                            let b = crate::object::convert_to_set(&args[1])?;
                            let mut res = crate::object::PySet::new();
                            for item in a.to_vec() {
                                if b.contains(&item).unwrap_or(false) { res.add(item)?; }
                            }
                            Ok(PyObjectRef::new(PyObject::Set(res)))
                        }),
                        "__or__" => Some(|args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                            if args.len() < 2 { return Err(PyError::type_error("__or__ missing args")); }
                            let a = crate::object::convert_to_set(&args[0])?;
                            let b = crate::object::convert_to_set(&args[1])?;
                            let mut res = a.clone();
                            for item in b.to_vec() { res.add(item)?; }
                            Ok(PyObjectRef::new(PyObject::Set(res)))
                        }),
                        "__xor__" => Some(|args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                            if args.len() < 2 { return Err(PyError::type_error("__xor__ missing args")); }
                            let a = crate::object::convert_to_set(&args[0])?;
                            let b = crate::object::convert_to_set(&args[1])?;
                            let mut res = crate::object::PySet::new();
                            for item in a.to_vec() {
                                if !b.contains(&item).unwrap_or(false) { res.add(item)?; }
                            }
                            for item in b.to_vec() {
                                if !a.contains(&item).unwrap_or(false) { res.add(item)?; }
                            }
                            Ok(PyObjectRef::new(PyObject::Set(res)))
                        }),
                        "__le__" => Some(|args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                            if args.len() < 2 { return Err(PyError::type_error("__le__ missing args")); }
                            let a = crate::object::convert_to_set(&args[0])?;
                            let b = crate::object::convert_to_set(&args[1])?;
                            let le = a.to_vec().iter().all(|item| b.contains(item).unwrap_or(false));
                            Ok(py_bool(le))
                        }),
                        "__lt__" => Some(|args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                            if args.len() < 2 { return Err(PyError::type_error("__lt__ missing args")); }
                            let a = crate::object::convert_to_set(&args[0])?;
                            let b = crate::object::convert_to_set(&args[1])?;
                            if a.len() >= b.len() { return Ok(py_bool(false)); }
                            let le = a.to_vec().iter().all(|item| b.contains(item).unwrap_or(false));
                            Ok(py_bool(le))
                        }),
                        "__ge__" => Some(|args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                            if args.len() < 2 { return Err(PyError::type_error("__ge__ missing args")); }
                            let a = crate::object::convert_to_set(&args[0])?;
                            let b = crate::object::convert_to_set(&args[1])?;
                            let ge = b.to_vec().iter().all(|item| a.contains(item).unwrap_or(false));
                            Ok(py_bool(ge))
                        }),
                        "__gt__" => Some(|args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                            if args.len() < 2 { return Err(PyError::type_error("__gt__ missing args")); }
                            let a = crate::object::convert_to_set(&args[0])?;
                            let b = crate::object::convert_to_set(&args[1])?;
                            if a.len() <= b.len() { return Ok(py_bool(false)); }
                            let ge = b.to_vec().iter().all(|item| a.contains(item).unwrap_or(false));
                            Ok(py_bool(ge))
                        }),
                        "__eq__" => Some(|args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                            if args.len() < 2 { return Err(PyError::type_error("__eq__ missing args")); }
                            let a = crate::object::convert_to_set(&args[0])?;
                            let b = crate::object::convert_to_set(&args[1])?;
                            if a.len() != b.len() { return Ok(py_bool(false)); }
                            for item in a.to_vec() {
                                if !b.contains(&item).unwrap_or(false) { return Ok(py_bool(false)); }
                            }
                            Ok(py_bool(true))
                        }),
                        "__ne__" => Some(|args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                            if args.len() < 2 { return Err(PyError::type_error("__ne__ missing args")); }
                            let a = crate::object::convert_to_set(&args[0])?;
                            let b = crate::object::convert_to_set(&args[1])?;
                            if a.len() != b.len() { return Ok(py_bool(true)); }
                            for item in a.to_vec() {
                                if !b.contains(&item).unwrap_or(false) { return Ok(py_bool(true)); }
                            }
                            Ok(py_bool(false))
                        }),
                        _ => None,
                    };
                    if let Some(func) = set_op {
                        return Ok(PyObjectRef::new(PyObject::BuiltinFunction {
                            name: name.to_string(),
                            func,
                        }));
                    }
                }
                if type_name == "str" {
                    let dummy = PyObject::Str(compact_str::CompactString::new(""));
                    if let Ok(val) = dummy.get_attribute(name) {
                        return Ok(val);
                    }
                }
                Err(PyError::attribute_error(format!(
                    "type has no attribute '{}'",
                    name
                )))
            }
            PyObject::Instance { dict, typ } => {
                if name == "__dict__" {
                    // Return a copy of the instance's HashMap as a PyDict (no
                    // live view from here) — NATIVE_BACKING_KEY is internal
                    // bookkeeping (see native_backing_of) and must not leak
                    // into user-visible introspection.
                    let mut pd = PyDict::new();
                    for (k, v) in dict.iter() {
                        if k == NATIVE_BACKING_KEY {
                            continue;
                        }
                        let _ = pd.set(py_str(k), v.clone());
                    }
                    return Ok(PyObjectRef::new(PyObject::Dict(Box::new(pd))));
                }
                if name == "__weakref__" {
                    // __weakref__ slot exists but returns None by default
                    // A full implementation would return a WeakRef object if one exists
                    return Ok(py_none());
                }
                // If __slots__ is defined, verify the attribute is allowed
                if let Some(slots) = get_instance_slots(typ) {
                    if !slots.iter().any(|s| s == name) {
                        // Check if it's a class-level attribute (method, etc.) — those are always allowed
                        let typ_ref = typ.borrow();
                        let is_in_type = if let PyObject::Type {
                            dict: type_dict,
                            mro,
                            ..
                        } = &*typ_ref
                        {
                            type_dict.contains_key_str(&name)
                                || mro.iter().skip(1).any(|base| {
                                    if let PyObject::Type {
                                        dict: base_dict, ..
                                    } = &*base.borrow()
                                    {
                                        base_dict.contains_key_str(&name)
                                    } else {
                                        false
                                    }
                                })
                        } else {
                            false
                        };
                        if !is_in_type {
                            let type_name = get_type_name_for_instance(typ);
                            return Err(PyError::attribute_error(format!(
                                "'{}' object has no attribute '{}'",
                                type_name, name
                            )));
                        }
                    }
                }
                dict.get_str(&name).cloned().or_else(|| {
                    let typ_ref = typ.borrow();
                    if let PyObject::Type { dict: type_dict, mro, .. } = &*typ_ref {
                        type_dict.get_str(&name).cloned().or_else(|| {
                            for base in mro.iter().skip(1) {
                                if let PyObject::Type { dict: base_dict, .. } = &*base.borrow() {
                                    if let Some(val) = base_dict.get_str(&name) {
                                        return Some(val.clone());
                                    }
                                }
                            }
                            // Not overridden anywhere in the mro: for a class
                            // that transparently subclasses list/dict/str
                            // (`class Foo(list): ...`), delegate to the real
                            // native value's own attribute resolution. Its
                            // get_attribute returns a placeholder self_obj
                            // (the real binding normally happens wherever
                            // LOAD_ATTR was invoked, rebinding to whatever it
                            // was accessed on) — here that must be rebound to
                            // the native backing itself, not this instance,
                            // or mutations would target the placeholder. This
                            // must run BEFORE the generic dict-like fallback
                            // below, which would otherwise misinterpret the
                            // native backing's own dict entry as plain
                            // instance-attribute data.
                            if let Some(native) = dict.get(NATIVE_BACKING_KEY) {
                                // A deque subclass's `__copy__`/`copy()` must
                                // return a NEW instance of the SAME subclass
                                // (real CPython: `D('abc').__copy__()` is a
                                // `D`), not a raw deque — the generic native
                                // delegation below would rebind `self_obj` to
                                // the backing deque and build a plain deque.
                                if matches!(&*native.borrow(), PyObject::Deque { .. }) && (name == "__copy__" || name == "copy") {
                                    let typ_clone = typ.clone();
                                    let new_native = {
                                        let b = native.borrow();
                                        if let PyObject::Deque { data, maxlen } = &*b {
                                            py_deque(data.clone(), *maxlen)
                                        } else { unreachable!() }
                                    };
                                    return Some(PyObjectRef::new(PyObject::Closure(Rc::new(move |_args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                                        let mut new_dict = AttrMap::new();
                                        new_dict.insert(NATIVE_BACKING_KEY.to_string(), new_native.clone());
                                        Ok(PyObjectRef::new(PyObject::Instance { typ: typ_clone.clone(), dict: new_dict }))
                                    }))));
                                }
                                if name == "__buffer__" || name == "__release_buffer__" {
                                    // Skip delegation for buffer protocol - let VM handle it
                                } else if let Ok(val) = native.borrow().get_attribute(name) {
                                    let rebound = if let PyObject::BuiltinMethod { name: n, func, .. } = &*val.borrow() {
                                        PyObjectRef::imm(PyObject::BuiltinMethod { name: n.clone(), func: *func, self_obj: native.clone() })
                                    } else {
                                        val.clone()
                                    };
                                    return Some(rebound);
                                }
                            }
                            // Fallback: provide common dict methods for dict-like instances
                            // Exclude dict view types which store mapping/kind_name
                            if (name == "__iter__" || name == "items" || name == "keys" || name == "values")
                                && !matches!(
                                    get_type_name_for_instance(typ).as_str(),
                                    "dict_items" | "dict_keys" | "dict_values"
                                )
                            {
                                let dict_snapshot: Vec<(String, PyObjectRef)> = dict.iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
                                let result = instance_builtin_dict_method(name, dict_snapshot);
                                return result;
                            }
                            // PEP 3134 traceback/chaining protocol methods and
                            // attributes for a USER-DEFINED exception class
                            // (`class MyError(Exception): ...`) that doesn't
                            // override them itself — the native
                            // `PyObject::Exception` representation already
                            // has these (see its own `get_attribute_impl`
                            // arm), but a custom subclass is a plain
                            // `PyObject::Instance` and fell straight through
                            // to `AttributeError` for all of them. Real
                            // trigger: `unittest`'s own `assertRaises`
                            // (`_AssertRaisesBaseContext.__exit__`) calling
                            // `exc_value.with_traceback(None)` on WHATEVER
                            // exception it caught — this raised
                            // `AttributeError` for literally any
                            // user-defined exception class, only working by
                            // accident for the handful of natively-
                            // represented ones.
                            if matches!(name, "with_traceback" | "add_note" | "__traceback__" | "__context__" | "__cause__" | "__suppress_context__" | "__notes__")
                                && find_exception_base_name(typ).is_some() {
                                return Some(match name {
                                    "with_traceback" => PyObjectRef::imm(PyObject::BuiltinMethod {
                                        name: "with_traceback".to_string(),
                                        func: |args| {
                                            if args.len() < 2 { return Err(PyError::type_error("with_traceback() takes exactly one argument")); }
                                            Ok(args[0].clone())
                                        },
                                        self_obj: PyObjectRef::new(PyObject::None),
                                    }),
                                    "add_note" => PyObjectRef::imm(PyObject::BuiltinMethod {
                                        name: "add_note".to_string(),
                                        func: |_args| Ok(py_none()),
                                        self_obj: PyObjectRef::new(PyObject::None),
                                    }),
                                    // `__cause__` was missing from this list entirely
                                    // (only `__context__`/`__traceback__` had a
                                    // fallback) — any user-defined exception class
                                    // reading its own `.__cause__` before ever
                                    // setting it (e.g. `raise X from Y` wasn't used)
                                    // raised `AttributeError` instead of `None`. Real
                                    // trigger: CPython's own doctest/exception-group
                                    // test files reading `.__cause__` on a plain
                                    // user-defined exception instance.
                                    "__context__" | "__traceback__" | "__cause__" => py_none(),
                                    "__suppress_context__" => py_bool(false),
                                    "__notes__" => py_list(vec![]),
                                    _ => unreachable!(),
                                });
                            }
                            None
                        })
                    } else {
                        None
                    }
                }).ok_or_else(|| PyError::attribute_error(format!("'{}' object has no attribute '{}'", get_type_name_for_instance(typ), name)))
            }
            PyObject::Property(ref d) => {
                let getter = &d.getter;
                let setter = &d.setter;
                let deleter = &d.deleter;
                let doc = &d.doc;
                match name {
                    "fget" => getter.clone().ok_or_else(|| {
                        PyError::attribute_error("property has no getter".to_string())
                    }),
                    "fset" => setter.clone().ok_or_else(|| {
                        PyError::attribute_error("property has no setter".to_string())
                    }),
                    "fdel" => deleter.clone().ok_or_else(|| {
                        PyError::attribute_error("property has no deleter".to_string())
                    }),
                    "doc" | "__doc__" => Ok(doc.clone().map_or_else(py_none, |d| py_str(&d))),
                    "__get__" => {
                        if let Some(_) = getter {
                            Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                                name: "__get__".to_string(),
                                func: |args| {
                                    if args.len() < 4 {
                                        return Err(PyError::type_error(
                                            "__get__() takes 2 positional arguments",
                                        ));
                                    }
                                    // args: [self_obj, descriptor, instance, owner]
                                    let g = args[1].borrow();
                                    if let PyObject::Property(ref data) = &*g {
                                        if let Some(ref getter_fn) = data.getter {
                                            call_bound_method(
                                                getter_fn.clone(),
                                                args[2].clone(),
                                                vec![],
                                            )
                                        } else {
                                            Err(PyError::runtime_error("property has no getter"))
                                        }
                                    } else {
                                        Err(PyError::runtime_error("property has no getter"))
                                    }
                                },
                                self_obj: PyObjectRef::new(PyObject::None),
                            }))
                        } else {
                            Err(PyError::attribute_error(
                                "property has no getter".to_string(),
                            ))
                        }
                    }
                    "__set__" => {
                        if let Some(_) = setter {
                            Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                                name: "__set__".to_string(),
                                func: |args| {
                                    if args.len() < 4 {
                                        return Err(PyError::type_error(
                                            "__set__() takes 2 positional arguments",
                                        ));
                                    }
                                    // args: [self_obj, descriptor, instance, value]
                                    let s = args[1].borrow();
                                    if let PyObject::Property(ref data) = &*s {
                                        if let Some(ref setter_fn) = data.setter {
                                            call_bound_method(
                                                setter_fn.clone(),
                                                args[2].clone(),
                                                vec![args[3].clone()],
                                            )
                                        } else {
                                            Err(PyError::runtime_error("property has no setter"))
                                        }
                                    } else {
                                        Err(PyError::runtime_error("property has no setter"))
                                    }
                                },
                                self_obj: PyObjectRef::new(PyObject::None),
                            }))
                        } else {
                            Err(PyError::attribute_error(
                                "property has no setter".to_string(),
                            ))
                        }
                    }
                    "setter" | "deleter" | "getter" => {
                        let is_setter = name == "setter";
                        let prop_obj = PyObjectRef::new(match o {
                            PyObject::Property(ref d) => {
                                PyObject::Property(Box::new(PropertyData {
                                    getter: d.getter.clone(),
                                    setter: d.setter.clone(),
                                    deleter: d.deleter.clone(),
                                    doc: d.doc.clone(),
                                }))
                            }
                            _ => unreachable!(),
                        });
                        Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                            name: name.to_string(),
                            func: if is_setter {
                                builtin_property_setter_fn
                            } else {
                                builtin_property_deleter_fn
                            },
                            self_obj: prop_obj,
                        }))
                    }
                    // `property.__isabstractmethod__` — real Python's ABC
                    // machinery (`abc.update_abstractmethods`, `ABCMeta`
                    // itself) checks this to recognize `@property
                    // @abstractmethod def foo(self): ...`-style abstract
                    // properties; missing entirely raised `AttributeError`
                    // for even the most basic ABC property test (real
                    // trigger: CPython's own `test_abc.py`'s
                    // `test_abstractproperty_basics`). True iff ANY of
                    // getter/setter/deleter is itself marked abstract,
                    // matching real CPython's own `property.__isabstractmethod__`.
                    "__isabstractmethod__" => {
                        fn is_abstract(f: &Option<PyObjectRef>) -> bool {
                            f.as_ref()
                                .and_then(|func| {
                                    func.borrow().get_attribute("__isabstractmethod__").ok()
                                })
                                .map(|v| v.truthy())
                                .unwrap_or(false)
                        }
                        Ok(py_bool(
                            is_abstract(getter) || is_abstract(setter) || is_abstract(deleter),
                        ))
                    }
                    _ => Err(PyError::attribute_error(format!(
                        "'property' object has no attribute '{}'",
                        name
                    ))),
                }
            }
            // `classmethod`/`staticmethod` had NO dedicated attribute-access
            // arm at all — any attribute access (`.__func__`,
            // `.__isabstractmethod__`) fell through to a generic
            // "not callable"/catch-all failure. `__isabstractmethod__` is
            // the one real trigger found (CPython's own `test_abc.py`'s
            // `test_abstractclassmethod_basics`/`test_abstractstaticmethod_basics`);
            // `__func__` (the real CPython attribute exposing the wrapped
            // function) added alongside it since it's the same shape of gap
            // and trivial to expose from the same field.
            PyObject::StaticMethod { func } => {
                match name {
                    "__func__" => Ok(func.clone()),
                    "__wrapped__" => Ok(func.clone()),
                    "__isabstractmethod__" => Ok(py_bool(
                        func.borrow()
                            .get_attribute("__isabstractmethod__")
                            .map(|v| v.truthy())
                            .unwrap_or(false),
                    )),
                    // `__name__`/`__module__`/`__qualname__`/`__doc__`/
                    // `__annotations__`/`__dict__` all delegate to the
                    // wrapped callable (test_decorators.py's
                    // check_wrapper_attrs asserts them on the descriptor).
                    _ => func.borrow().get_attribute(name).map_err(|_| {
                        PyError::attribute_error(format!(
                            "'staticmethod' object has no attribute '{}'",
                            name
                        ))
                    }),
                }
            }
            PyObject::ClassMethod { func } => match name {
                "__func__" => Ok(func.clone()),
                "__wrapped__" => Ok(func.clone()),
                "__isabstractmethod__" => Ok(py_bool(
                    func.borrow()
                        .get_attribute("__isabstractmethod__")
                        .map(|v| v.truthy())
                        .unwrap_or(false),
                )),
                _ => func.borrow().get_attribute(name).map_err(|_| {
                    PyError::attribute_error(format!(
                        "'classmethod' object has no attribute '{}'",
                        name
                    ))
                }),
            },
        _ => unreachable!("{} called with non-{} object", stringify!(get), o.type_name()),
    }
}
