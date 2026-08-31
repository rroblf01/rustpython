use crate::object::*;
use std::cell::RefCell;

// ============================================================
// contextvars — real Context/ContextVar/Token semantics
// ============================================================
//
// Real CPython's own `contextvars.py` is a 7-line wrapper around a C
// extension (`_contextvars`) backed by an immutable HAMT for O(log n)
// structural-sharing copies. There is no pure-Python reference
// implementation to vendor here (unlike `abc`/`collections.abc` earlier
// this session) — this is genuine native implementation work. The
// underlying data structure does NOT need to replicate CPython's HAMT
// performance characteristic: a plain `PyDict` cloned on `.copy()` is a
// correct, sufficient implementation — what's actually tested is the
// *semantics* (isolation between contexts, token validity, reentrancy
// detection), not big-O.
//
// `_ContextRaw` is a plain native type (no ABC machinery of its own)
// providing `__getitem__`/`__iter__`/`__len__`/`run`/`copy`. The PUBLIC
// `Context` name is built via a tiny `install_source_defined_stdlib`
// snippet (see `src/vm.rs`) as `class Context(_ContextRaw,
// collections.abc.Mapping): pass` — ordinary multiple inheritance through
// the already-working general class-creation/MRO machinery gives every
// other Mapping method (`get`/`keys`/`values`/`items`/`__contains__`/
// `__eq__`/`__ne__`) for free from `_collections_abc.py`'s real mixins,
// matching real CPython's own `_collections_abc.Mapping.register(Context)`
// in spirit (virtual registration there; real inheritance here — real
// CPython can't inherit from a Python ABC from a C type the way we can
// from Rust, but the observable Mapping-protocol behavior is the same).

thread_local! {
    /// Stack of currently-active `Context` instances (top = current).
    /// Lazily seeded with an implicit default context on first access,
    /// matching real CPython: a `ContextVar.get()/.set()` outside any
    /// `Context.run()` call still operates against SOME context.
    static CONTEXT_STACK: RefCell<Vec<PyObjectRef>> = RefCell::new(Vec::new());
    static MISSING_SENTINEL: RefCell<Option<PyObjectRef>> = const { RefCell::new(None) };
    static CONTEXTVAR_TYPE: RefCell<Option<PyObjectRef>> = const { RefCell::new(None) };
    static TOKEN_TYPE: RefCell<Option<PyObjectRef>> = const { RefCell::new(None) };
    static CONTEXT_RAW_TYPE: RefCell<Option<PyObjectRef>> = const { RefCell::new(None) };
    /// The real, public `contextvars.Context` type (`class Context
    /// (_ContextRaw, collections.abc.Mapping): pass`, built once
    /// `collections.abc` is available — see `stamp_no_subclass`'s caller in
    /// `src/vm.rs`). Every `Context` instance constructed after this is set
    /// should carry this MRO (Mapping's mixins included), not the bare
    /// `_ContextRaw` used only as a transitional base before it exists.
    static PUBLIC_CONTEXT_TYPE: RefCell<Option<PyObjectRef>> = const { RefCell::new(None) };
}

/// Called once from `src/vm.rs` right after `class Context(_ContextRaw,
/// collections.abc.Mapping): pass` is built, so every `Context` instance
/// constructed from here on (the lazily-created default context,
/// `copy_context()`, `.copy()`) carries the real, Mapping-inheriting type
/// instead of the bare native `_ContextRaw`.
pub(crate) fn set_public_context_type(cls: PyObjectRef) {
    PUBLIC_CONTEXT_TYPE.with(|c| *c.borrow_mut() = Some(cls));
}

/// The type new `Context` instances should be built with: the real public
/// `Context` (once available) or `_ContextRaw` itself as a bootstrap
/// fallback for any (unexpected) contextvars use before `set_
/// public_context_type` runs.
fn context_instance_type() -> PyObjectRef {
    PUBLIC_CONTEXT_TYPE
        .with(|c| c.borrow().clone())
        .unwrap_or_else(context_raw_type)
}

/// A single canonical marker instance so identity comparison (`is`) works
/// reliably for `Token.MISSING`, regardless of how string/other literal
/// values happen to be interned/inlined elsewhere in the object model.
fn missing_sentinel() -> PyObjectRef {
    MISSING_SENTINEL.with(|c| {
        let mut b = c.borrow_mut();
        if b.is_none() {
            let marker_type = PyObjectRef::new(PyObject::Type {
                name: "_TokenMissingType".to_string(),
                dict: Box::new(TypeDict::default()),
                bases: vec![],
                mro: vec![],
            });
            *b = Some(PyObjectRef::new(PyObject::Instance {
                typ: marker_type,
                dict: AttrMap::new(),
            }));
        }
        b.clone().unwrap()
    })
}

fn contextvar_type() -> PyObjectRef {
    CONTEXTVAR_TYPE
        .with(|c| c.borrow().clone())
        .expect("ContextVar type not initialized")
}

fn token_type() -> PyObjectRef {
    TOKEN_TYPE
        .with(|c| c.borrow().clone())
        .expect("Token type not initialized")
}

fn context_raw_type() -> PyObjectRef {
    CONTEXT_RAW_TYPE
        .with(|c| c.borrow().clone())
        .expect("_ContextRaw type not initialized")
}

fn is_instance_of(v: &PyObjectRef, typ: &PyObjectRef) -> bool {
    matches!(&*v.borrow(), PyObject::Instance { typ: t, .. } if t.is(typ))
}

/// Sets `bases`/`mro` to a plain `[self, object]` chain — needed for
/// anything built directly as a `PyObject::Type` in Rust (bypassing the
/// normal `default_build_class` path, which auto-adds `object` as a base
/// when none is given): without this, MRO-dependent generic mechanisms
/// like `.hash()`'s `__hash__` lookup (which needs to find `object`'s own
/// default, identity-based implementation) find nothing and wrongly report
/// the type as unhashable. `object` itself is NOT in `get_primitive_type`'s
/// cache (only int/str/list/etc. seed that — `object`/`type` are handled
/// specially elsewhere), so it's threaded in explicitly from `builtins`
/// (see `create_contextvars_dict`'s own signature) rather than looked up
/// here.
fn attach_object_base(typ: &PyObjectRef, object_type: &PyObjectRef) {
    if let PyObject::Type { bases, mro, .. } = &mut *typ.borrow_mut() {
        *bases = vec![object_type.clone()];
        *mro = vec![typ.clone(), object_type.clone()];
    }
}

/// Accepts a real `str`, a `SmallStr`, or a native-backed `str` subclass
/// instance (e.g. `class weird_str(str): ...`) — matches `isinstance(x,
/// str)`'s breadth, not just the exact builtin type.
fn is_str_like(v: &PyObjectRef) -> bool {
    if matches!(&*v.borrow(), PyObject::Str(_)) {
        return true;
    }
    if let Some(backing) = crate::object::native_backing_of(v) {
        return matches!(&*backing.borrow(), PyObject::Str(_));
    }
    false
}

fn new_context_raw() -> PyObjectRef {
    let mut dict = AttrMap::new();
    dict.insert_str("_data", PyObjectRef::new(PyObject::Dict(Box::new(PyDict::new()))));
    PyObjectRef::new(PyObject::Instance {
        typ: context_instance_type(),
        dict,
    })
}

/// The context at the top of the stack, lazily creating an implicit
/// default one if nothing has been entered via `.run()` yet.
fn current_context() -> PyObjectRef {
    CONTEXT_STACK.with(|s| {
        let mut stack = s.borrow_mut();
        if stack.is_empty() {
            stack.push(new_context_raw());
        }
        stack.last().unwrap().clone()
    })
}

fn context_data(ctx: &PyObjectRef) -> PyObjectRef {
    if let PyObject::Instance { dict, .. } = &*ctx.borrow() {
        dict.get_str("_data").expect("Context missing _data").clone()
    } else {
        unreachable!("not a _ContextRaw instance")
    }
}

fn data_get_by_identity(data: &PyObjectRef, key: &PyObjectRef) -> Option<PyObjectRef> {
    if let PyObject::Dict(d) = &*data.borrow() {
        d.get_by_identity(key)
    } else {
        None
    }
}

fn data_set_by_identity(data: &PyObjectRef, key: PyObjectRef, value: PyObjectRef) {
    if let PyObject::Dict(d) = &mut *data.borrow_mut() {
        d.set_by_identity(key, value);
    }
}

fn data_remove_by_identity(data: &PyObjectRef, key: &PyObjectRef) {
    // No `remove_by_identity` exists on `PyDict` (`.remove()` hashes/
    // compares via `.equals()`, wrong for keying by object identity) —
    // rebuild the dict without the matching entry instead. Correctness
    // over micro-efficiency: a Context's own var count is always tiny.
    let remaining: Vec<(PyObjectRef, PyObjectRef)> = if let PyObject::Dict(d) = &*data.borrow() {
        d.items().into_iter().filter(|(k, _)| !k.is(key)).collect()
    } else {
        return;
    };
    let mut fresh = PyDict::new();
    for (k, v) in remaining {
        fresh.set_by_identity(k, v);
    }
    if let PyObject::Dict(d) = &mut *data.borrow_mut() {
        **d = fresh;
    }
}

fn data_len(data: &PyObjectRef) -> usize {
    if let PyObject::Dict(d) = &*data.borrow() {
        d.len()
    } else {
        0
    }
}

fn data_keys(data: &PyObjectRef) -> Vec<PyObjectRef> {
    if let PyObject::Dict(d) = &*data.borrow() {
        d.keys()
    } else {
        Vec::new()
    }
}

fn context_var_repr(var: &PyObjectRef) -> String {
    if let PyObject::Instance { dict, .. } = &*var.borrow() {
        let name = dict.get_str("_name").map(|v| v.str()).unwrap_or_default();
        let has_default = dict
            .get_str("_has_default")
            .map(|v| v.truthy())
            .unwrap_or(false);
        if has_default {
            let default = dict.get_str("_default").cloned().unwrap_or_else(py_none);
            format!(
                "<ContextVar name='{}' default={} at {:#x}>",
                name,
                default.repr(),
                var.get_id()
            )
        } else {
            format!("<ContextVar name='{}' at {:#x}>", name, var.get_id())
        }
    } else {
        "<ContextVar>".to_string()
    }
}

fn token_repr(token: &PyObjectRef) -> String {
    if let PyObject::Instance { dict, .. } = &*token.borrow() {
        let used = dict
            .get_str("_used")
            .map(|v| v.truthy())
            .unwrap_or(false);
        let var = dict.get_str("_var").cloned().unwrap_or_else(py_none);
        if used {
            format!(
                "<Token used var={} at {:#x}>",
                context_var_repr(&var),
                token.get_id()
            )
        } else {
            format!(
                "<Token var={} at {:#x}>",
                context_var_repr(&var),
                token.get_id()
            )
        }
    } else {
        "<Token>".to_string()
    }
}

/// `run()`'s own registered `BuiltinFunction` — the real dispatch happens
/// via identity match in `src/vm/call.rs::try_handle_special_builtin`
/// (needs live `&mut VirtualMachine` access to call the wrapped callable
/// with its own real positional/keyword arguments, and to push/pop the
/// context stack around it). This body is a defensive fallback that should
/// never actually run in practice — every call to a bound `ctx.run` method
/// routes through `call_function`, which always checks
/// `try_handle_special_builtin` first.
pub(crate) fn context_run_fallback(_args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    Err(PyError::runtime_error(
        "Context.run() called without VM dispatch (internal error)",
    ))
}

/// The real implementation behind `Context.run(callable, *args, **kwargs)`,
/// invoked from `try_handle_special_builtin` with genuine `&mut
/// VirtualMachine` access.
pub(crate) fn context_run_with_vm(
    vm: &mut crate::vm::VirtualMachine,
    ctx: PyObjectRef,
    args: &[PyObjectRef],
    keywords: &[(String, PyObjectRef)],
) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error(
            "run() missing 1 required positional argument: 'callable'",
        ));
    }
    let callable = args[0].clone();
    let fwd_args = args[1..].to_vec();
    let fwd_kwargs = keywords.to_vec();

    let already_entered = CONTEXT_STACK.with(|s| s.borrow().iter().any(|c| c.is(&ctx)));
    if already_entered {
        return Err(PyError::runtime_error(format!(
            "cannot enter context: {} is already entered",
            ctx.repr()
        )));
    }
    CONTEXT_STACK.with(|s| s.borrow_mut().push(ctx.clone()));
    let result = vm.call_function(callable, fwd_args, fwd_kwargs);
    CONTEXT_STACK.with(|s| {
        s.borrow_mut().pop();
    });
    result
}

/// Extracts a trailing-dict-packed keyword argument by name (this
/// codebase's established convention for `BuiltinFunction` calls — see
/// `call_function`'s own doc comments), or `None` if absent.
fn kwarg(args: &[PyObjectRef], name: &str) -> Option<PyObjectRef> {
    let last = args.last()?;
    if let PyObject::Dict(d) = &*last.borrow() {
        d.get(&py_str(name)).ok().flatten()
    } else {
        None
    }
}

fn positional_len(args: &[PyObjectRef]) -> usize {
    match args.last() {
        Some(last) if matches!(&*last.borrow(), PyObject::Dict(_)) => args.len() - 1,
        _ => args.len(),
    }
}

pub fn create_contextvars_dict(object_type: PyObjectRef) -> std::collections::HashMap<String, PyObjectRef> {
    let mut d = std::collections::HashMap::new();

    macro_rules! method {
        ($map:expr, $name:expr, $func:expr) => {
            $map.insert_str(
                $name,
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: $name.to_string(),
                    func: $func,
                }),
            );
        };
    }

    // ---------------- ContextVar ----------------
    let mut cv_dict: TypeDict = Default::default();

    method!(cv_dict, "__init__", |args: &[PyObjectRef]| {
        if args.is_empty() {
            return Err(PyError::type_error(
                "ContextVar() takes exactly 1 positional argument (0 given)",
            ));
        }
        let n_positional = positional_len(&args[1..]);
        if n_positional != 1 {
            return Err(PyError::type_error(format!(
                "ContextVar() takes exactly 1 positional argument ({} given)",
                n_positional
            )));
        }
        let name = &args[1];
        if !is_str_like(name) {
            return Err(PyError::type_error("context variable name must be a str"));
        }
        // Real CPython hashes the name internally (HAMT key material) —
        // replicate the observable behavior (a genuinely unhashable name,
        // e.g. a `str` subclass overriding `__eq__` without `__hash__`,
        // must raise `TypeError: unhashable type`) even though this
        // implementation doesn't otherwise need the hash for anything.
        name.hash()?;
        let (has_default, default) = match kwarg(args, "default") {
            Some(v) => (true, v),
            None => (false, py_none()),
        };
        if let PyObject::Instance { dict, .. } = &mut *args[0].borrow_mut() {
            dict.insert_str("_name", name.clone());
            dict.insert_str("_has_default", py_bool(has_default));
            dict.insert_str("_default", default);
        }
        Ok(py_none())
    });

    {
        let getter = PyObjectRef::new(PyObject::BuiltinFunction {
            name: "name".to_string(),
            func: |args: &[PyObjectRef]| {
                if let PyObject::Instance { dict, .. } = &*args[0].borrow() {
                    if let Some(v) = dict.get_str("_name") {
                        return Ok(v.clone());
                    }
                }
                Err(PyError::type_error("ContextVar instance has no name"))
            },
        });
        cv_dict.insert_str(
            "name",
            PyObjectRef::new(PyObject::Property(Box::new(PropertyData {
                getter: Some(getter),
                setter: None,
                deleter: None,
                doc: None,
            }))),
        );
    }

    method!(cv_dict, "get", |args: &[PyObjectRef]| {
        let this = &args[0];
        let ctx = current_context();
        let data = context_data(&ctx);
        if let Some(v) = data_get_by_identity(&data, this) {
            return Ok(v);
        }
        let n_positional = positional_len(&args[1..]);
        if n_positional >= 1 {
            return Ok(args[1].clone());
        }
        let (has_default, default) = if let PyObject::Instance { dict, .. } = &*this.borrow() {
            (
                dict.get_str("_has_default")
                    .map(|v| v.truthy())
                    .unwrap_or(false),
                dict.get_str("_default").cloned().unwrap_or_else(py_none),
            )
        } else {
            (false, py_none())
        };
        if has_default {
            Ok(default)
        } else {
            Err(PyError::key_error(context_var_repr(this)))
        }
    });

    method!(cv_dict, "set", |args: &[PyObjectRef]| {
        if args.len() < 2 {
            return Err(PyError::type_error(
                "set() missing 1 required positional argument: 'value'",
            ));
        }
        let this = args[0].clone();
        let value = args[1].clone();
        let ctx = current_context();
        let data = context_data(&ctx);
        let old_value = data_get_by_identity(&data, &this).unwrap_or_else(missing_sentinel);
        data_set_by_identity(&data, this.clone(), value);

        let mut token_dict = AttrMap::new();
        token_dict.insert_str("_var", this);
        token_dict.insert_str("_old_value", old_value);
        token_dict.insert_str("_context", ctx);
        token_dict.insert_str("_used", py_bool(false));
        Ok(PyObjectRef::new(PyObject::Instance {
            typ: token_type(),
            dict: token_dict,
        }))
    });

    method!(cv_dict, "reset", |args: &[PyObjectRef]| {
        if args.len() < 2 {
            return Err(PyError::type_error(
                "reset() missing 1 required positional argument: 'token'",
            ));
        }
        let this = &args[0];
        let token = &args[1];
        if !is_instance_of(token, &token_type()) {
            return Err(PyError::type_error("reset() argument must be a Token"));
        }
        let (used, token_var, old_value, token_ctx) =
            if let PyObject::Instance { dict, .. } = &*token.borrow() {
                (
                    dict.get_str("_used").map(|v| v.truthy()).unwrap_or(false),
                    dict.get_str("_var").cloned().unwrap_or_else(py_none),
                    dict.get_str("_old_value").cloned().unwrap_or_else(py_none),
                    dict.get_str("_context").cloned().unwrap_or_else(py_none),
                )
            } else {
                return Err(PyError::type_error("reset() argument must be a Token"));
            };
        if used {
            return Err(PyError::runtime_error(format!(
                "{} has already been used once",
                token_repr(token)
            )));
        }
        if !token_var.is(this) {
            return Err(PyError::value_error(format!(
                "{} was created by a different ContextVar",
                token_repr(token)
            )));
        }
        let ctx = current_context();
        if !token_ctx.is(&ctx) {
            return Err(PyError::value_error(format!(
                "{} was created in a different Context",
                token_repr(token)
            )));
        }
        let data = context_data(&ctx);
        if old_value.is(&missing_sentinel()) {
            data_remove_by_identity(&data, this);
        } else {
            data_set_by_identity(&data, this.clone(), old_value);
        }
        if let PyObject::Instance { dict, .. } = &mut *token.borrow_mut() {
            dict.insert_str("_used", py_bool(true));
        }
        Ok(py_none())
    });

    method!(cv_dict, "__repr__", |args: &[PyObjectRef]| {
        Ok(py_str(&context_var_repr(&args[0])))
    });

    let contextvar_type_obj = PyObjectRef::new(PyObject::Type {
        name: "ContextVar".to_string(),
        dict: Box::new(cv_dict),
        bases: vec![],
        mro: vec![],
    });
    attach_object_base(&contextvar_type_obj, &object_type);
    // Real CPython disallows subclassing `ContextVar`/`Context`/`Token`
    // (`TypeError: type '...' is not an acceptable base type` —
    // `test_context.py::test_context_subclassing_1`).
    if let PyObject::Type { dict, .. } = &mut *contextvar_type_obj.borrow_mut() {
        dict.insert_str(crate::object::NO_SUBCLASS_KEY, py_bool(true));
    }
    CONTEXTVAR_TYPE.with(|c| *c.borrow_mut() = Some(contextvar_type_obj.clone()));

    // ---------------- Token ----------------
    let mut token_dict_type: TypeDict = Default::default();
    for (name, getter_key) in [("var", "_var"), ("old_value", "_old_value")] {
        let key = getter_key.to_string();
        let getter = PyObjectRef::new(PyObject::Closure(std::rc::Rc::new(move |args| {
            if let PyObject::Instance { dict, .. } = &*args[0].borrow() {
                if let Some(v) = dict.get_str(&key) {
                    return Ok(v.clone());
                }
            }
            Err(PyError::type_error("Token instance missing internal state"))
        })));
        token_dict_type.insert_str(
            name,
            PyObjectRef::new(PyObject::Property(Box::new(PropertyData {
                getter: Some(getter),
                setter: None,
                deleter: None,
                doc: None,
            }))),
        );
    }
    method!(token_dict_type, "__repr__", |args: &[PyObjectRef]| {
        Ok(py_str(&token_repr(&args[0])))
    });
    method!(token_dict_type, "__enter__", |args: &[PyObjectRef]| {
        Ok(args[0].clone())
    });
    method!(token_dict_type, "__exit__", |args: &[PyObjectRef]| {
        let this = args[0].clone();
        let var = if let PyObject::Instance { dict, .. } = &*this.borrow() {
            dict.get_str("_var").cloned().unwrap_or_else(py_none)
        } else {
            py_none()
        };
        let reset = var.borrow().get_attribute("reset").ok();
        if let Some(reset_fn) = reset {
            crate::object::call_bound_method(reset_fn, var, vec![this])?;
        }
        Ok(py_bool(false))
    });
    token_dict_type.insert_str("MISSING", missing_sentinel());

    let token_type_obj = PyObjectRef::new(PyObject::Type {
        name: "Token".to_string(),
        dict: Box::new(token_dict_type),
        bases: vec![],
        mro: vec![],
    });
    attach_object_base(&token_type_obj, &object_type);
    if let PyObject::Type { dict, .. } = &mut *token_type_obj.borrow_mut() {
        dict.insert_str(crate::object::NO_SUBCLASS_KEY, py_bool(true));
    }
    TOKEN_TYPE.with(|c| *c.borrow_mut() = Some(token_type_obj.clone()));

    // ---------------- _ContextRaw (Context's native half) ----------------
    let mut ctx_dict: TypeDict = Default::default();

    method!(ctx_dict, "__init__", |args: &[PyObjectRef]| {
        let has_extra_positional = positional_len(&args[1..]) > 0;
        let has_extra_kwargs = matches!(args.last(), Some(l) if matches!(&*l.borrow(), PyObject::Dict(d) if !d.is_empty()));
        if has_extra_positional || has_extra_kwargs {
            return Err(PyError::type_error("Context() does not accept any arguments"));
        }
        if let PyObject::Instance { dict, .. } = &mut *args[0].borrow_mut() {
            dict.insert_str("_data", PyObjectRef::new(PyObject::Dict(Box::new(PyDict::new()))));
        }
        Ok(py_none())
    });

    method!(ctx_dict, "__getitem__", |args: &[PyObjectRef]| {
        let key = &args[1];
        if !is_instance_of(key, &contextvar_type()) {
            return Err(PyError::type_error(format!(
                "a ContextVar key was expected, got {}",
                key.repr()
            )));
        }
        let data = context_data(&args[0]);
        data_get_by_identity(&data, key).ok_or_else(|| PyError::key_error_obj(key))
    });

    method!(ctx_dict, "__len__", |args: &[PyObjectRef]| {
        Ok(py_int(data_len(&context_data(&args[0])) as i64))
    });

    method!(ctx_dict, "__iter__", |args: &[PyObjectRef]| {
        let keys = data_keys(&context_data(&args[0]));
        crate::object::builtin_iter(&[py_list(keys)])
    });

    method!(ctx_dict, "copy", |args: &[PyObjectRef]| {
        let data = context_data(&args[0]);
        let cloned = if let PyObject::Dict(d) = &*data.borrow() {
            let mut fresh = PyDict::new();
            for (k, v) in d.items() {
                fresh.set_by_identity(k, v);
            }
            fresh
        } else {
            PyDict::new()
        };
        let mut dict = AttrMap::new();
        dict.insert_str("_data", PyObjectRef::new(PyObject::Dict(Box::new(cloned))));
        Ok(PyObjectRef::new(PyObject::Instance {
            typ: context_instance_type(),
            dict,
        }))
    });

    // `run` is registered here as a plain marker/fallback; the REAL
    // dispatch (needing live `&mut VirtualMachine` access) happens in
    // `src/vm/call.rs::try_handle_special_builtin`, matched by this exact
    // function pointer's identity.
    method!(ctx_dict, "run", context_run_fallback);

    method!(ctx_dict, "__repr__", |args: &[PyObjectRef]| {
        Ok(py_str(&format!(
            "<contextvars.Context object at {:#x}>",
            args[0].get_id()
        )))
    });

    let context_raw_type_obj = PyObjectRef::new(PyObject::Type {
        name: "_ContextRaw".to_string(),
        dict: Box::new(ctx_dict),
        bases: vec![],
        mro: vec![],
    });
    attach_object_base(&context_raw_type_obj, &object_type);
    CONTEXT_RAW_TYPE.with(|c| *c.borrow_mut() = Some(context_raw_type_obj.clone()));

    // ---------------- copy_context() ----------------
    method!(d, "copy_context", |_args: &[PyObjectRef]| {
        let ctx = current_context();
        let copy_fn = ctx.borrow().get_attribute("copy").ok();
        match copy_fn {
            Some(f) => crate::object::call_bound_method(f, ctx, vec![]),
            None => Ok(new_context_raw()),
        }
    });

    d.insert_str("ContextVar", contextvar_type_obj);
    d.insert_str("Token", token_type_obj);
    d.insert_str("_ContextRaw", context_raw_type_obj);
    d.insert_str("__name__", py_str("contextvars"));
    d.insert_str(
        "__doc__",
        py_str("Context Variables (PEP 567), native implementation"),
    );

    d
}
