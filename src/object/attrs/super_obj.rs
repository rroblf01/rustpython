// Auto-extracted from src/object/attrs/mod.rs lines 4483-4909
use crate::object::*;
use super::*;
use crate::interner;
use std::rc::Rc;

pub(crate) fn get(o: &PyObject, name: &str) -> PyResult<PyObjectRef> {
    match o {
            PyObject::Super { cls, obj } => {
                // super(cls, obj).attr: walk MRO of obj's type, starting after cls.
                // When `obj` is itself a class/type — the "classmethod-style"
                // form real Python uses for metaclass methods, e.g. inside a
                // metaclass's `def __new__(metacls, name, bases, ns):`, where
                // bare `super()` binds obj=metacls — the relevant mro is
                // `obj`'s own (e.g. a metaclass's own mro), not some further
                // "type of obj" (which would just be `type`/whatever built
                // it, an unrelated chain). Without this, `super().__new__(...)`
                // inside a metaclass's `__new__` couldn't resolve `__new__`
                // at all (AttributeError), since `obj` isn't a plain Instance
                // and has no meaningful `__class__` for this purpose either.
                let obj_types: Vec<PyObjectRef> = if let PyObject::Instance { typ, .. } = &*obj.borrow() {
                    vec![typ.clone()]
                } else if matches!(&*obj.borrow(), PyObject::Type { .. }) {
                    // For a Type obj, try its own MRO first (e.g. super() inside __new__ where obj is metacls)
                    // and if cls not found there, try its metatype's MRO (e.g. super() inside __init__ where obj is a class instance of the metaclass)
                    let mut v = vec![obj.clone()];
                    if let Some(mt) = crate::object::metatype_of(&obj) {
                        v.push(mt);
                    } else if let Ok(cls_attr) = obj.borrow().get_attribute("__class__") {
                        // fallback: type(obj)
                        if !v.iter().any(|x| x.is(&cls_attr)) {
                            v.push(cls_attr);
                        }
                    }
                    v
                } else {
                    match obj.borrow().get_attribute("__class__").ok() {
                        Some(c) => vec![c],
                        None => vec![],
                    }
                };
                for obj_type in obj_types {
                    if let PyObject::Type { mro, .. } = &*obj_type.borrow() {
                        if std::env::var("RPY_DEBUG_SUPER2").is_ok() {
                            let cls_name = cls.borrow().type_name().to_string();
                            let mro_names: Vec<String> =
                                mro.iter().map(|m| m.borrow().type_name()).collect();
                            eprintln!(
                                "SUPER2 cls={} obj_type={} mro={:?} name={}",
                                cls_name,
                                obj_type.borrow().type_name(),
                                mro_names,
                                name
                            );
                        }
                        // Find cls in MRO, start search from the next class.
                        // If `cls` isn't in `obj`'s MRO at all — e.g. a
                        // zero-arg `super()`'s compiled-in `LOAD_GLOBAL
                        // <ClassName>` (see compile_expr's PEP 3135 handling)
                        // picked up a DIFFERENT object than the class this
                        // method actually belongs to, because that global
                        // name got rebound/re-imported to something else in
                        // the meantime — `unwrap_or(0) + 1` used to silently
                        // treat "not found" as "found at position 0", i.e.
                        // start the search at `mro[1]`. For a method whose
                        // own class IS in `obj`'s real MRO (the overwhelmingly
                        // common case, just not reachable via this wrong
                        // `cls`), `mro[1]` is often that SAME class again —
                        // so `super().method()` calls itself again as if it
                        // were the next-in-MRO implementation, forever.
                        // Confirmed via a general, Django-free repro
                        // (rebinding a class's own name inside its
                        // `__init_subclass__` before the trailing
                        // `super().__init_subclass__()` call reproduces
                        // unbounded recursion). Real CPython raises
                        // `TypeError: super(type, obj): obj must be an
                        // instance or subtype of type` here instead — treat
                        // it as "not found via this MRO" and fall through to
                        // the native-backing/error handling below, which is
                        // at least a clean, immediate failure rather than a
                        // silent infinite loop.
                        // Real identity check via `.is()` — the previous
                        // hand-rolled match only ever compared two `Mut`
                        // variants (`Rc::ptr_eq`), silently returning
                        // `false` for anything else. Class/`Type` objects
                        // in this codebase are NOT guaranteed to be `Mut`
                        // (several are `Imm`), so `super(C, e)` — the
                        // EXPLICIT two-argument form, as opposed to the
                        // compiler-synthesized zero-arg one, which happened
                        // to always deal with `Mut` classes in whatever
                        // cases exercised it before — could never find
                        // `cls` in `obj`'s mro at all, making EVERY
                        // attribute lookup through such a `super()` object
                        // fail with `AttributeError`. Confirmed via
                        // CPython's own `test_super.py::test_pickling`
                        // (`s = super(C, e); s.f()`).
                        let start_idx = mro.iter().position(|m| cls.is(m)).map(|p| p + 1);
                        if let Some(start_idx) = start_idx {
                            if start_idx < mro.len() {
                                let mut found = None;
                                for base in mro.iter().skip(start_idx) {
                                    // A builtin exception base (`class MyError
                                    // (OSError): ...`) is a `PyObject::
                                    // BuiltinFunction` (the exception's own
                                    // constructor), never a real `PyObject::
                                    // Type` — invisible to the dict-lookup
                                    // walk just below, so `super().__init__
                                    // (...)` inside such a subclass's own
                                    // `__init__` always raised `AttributeError:
                                    // 'super' object has no attribute
                                    // '__init__'` instead of reaching real
                                    // `BaseException.__init__`'s behavior
                                    // (store the given args as `self.args`).
                                    // Extremely common idiom — any custom
                                    // exception hierarchy that calls
                                    // `super().__init__(...)` (real trigger:
                                    // `urllib.error.URLError(OSError)`).
                                    if name == "__init__" {
                                        if let PyObject::BuiltinFunction { name: bname, .. } =
                                            &*base.borrow()
                                        {
                                            if is_builtin_exception_class_name(bname) {
                                                let target = obj.clone();
                                                found = Some(PyObjectRef::new(PyObject::Closure(Rc::new(move |args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                                                if let PyObject::Instance { dict, .. } = &mut *target.borrow_mut() {
                                                    dict.insert("args".to_string(), py_tuple(args.to_vec()));
                                                }
                                                Ok(py_none())
                                            }))));
                                                break;
                                            }
                                        }
                                    }
                                    if let PyObject::Type { dict, .. } = &*base.borrow() {
                                        if let Some(val) = dict.get_str(&name) {
                                            let val_borrowed = val.borrow();
                                            match &*val_borrowed {
                                                // `__new__` is *always* implicitly
                                                // a staticmethod in real Python —
                                                // never auto-bound — regardless of
                                                // whether it's explicitly wrapped
                                                // in `staticmethod(...)`. Only the
                                                // explicit-wrapper case was
                                                // unwrapped below; a plain `def
                                                // __new__(mcs, ...):` (which is
                                                // how virtually every real
                                                // metaclass, including Django's,
                                                // writes it — nobody bothers with
                                                // `@staticmethod` there) still hit
                                                // the auto-bind arm just below,
                                                // producing a BoundMethod that
                                                // prepended `obj` as an EXTRA,
                                                // duplicate leading argument on
                                                // top of the one already passed
                                                // explicitly (`super().__new__(mcs,
                                                // name, bases, attrs)` always
                                                // passes `mcs` itself) — shifting
                                                // every subsequent positional arg
                                                // by one.
                                                PyObject::Function(_)
                                                | PyObject::BuiltinFunction { .. }
                                                    if name == "__new__" =>
                                                {
                                                    found = Some(val.clone());
                                                    break;
                                                }
                                                PyObject::Function(_)
                                                | PyObject::BuiltinFunction { .. } => {
                                                    found = Some(PyObjectRef::new(
                                                        PyObject::BoundMethod {
                                                            func: val.clone(),
                                                            self_obj: obj.clone(),
                                                        },
                                                    ));
                                                    break;
                                                }
                                                // A method found directly in a
                                                // migrated native type's own
                                                // dict (e.g. `dict.__setitem__`,
                                                // `dict.__getitem__`) is stored
                                                // as a `BuiltinMethod` with a
                                                // PLACEHOLDER `self_obj` (see
                                                // `NATIVE_VALUE_CTOR_KEY`'s doc
                                                // comment) — the catch-all arm
                                                // below returns such values
                                                // UNCHANGED, which is correct
                                                // for genuine bound instance
                                                // methods (their `self_obj` is
                                                // already the right target) but
                                                // WRONG here: this placeholder
                                                // must be rebound to `obj` (the
                                                // real instance `super()` was
                                                // constructed for), exactly like
                                                // the `Function`/`BuiltinFunction`
                                                // case just above. Missing this
                                                // meant `super().__setitem__(k, v)`
                                                // inside e.g. `enum.py`'s
                                                // `_EnumDict.__setitem__`
                                                // resolved to `dict.__setitem__`
                                                // with its self_obj STILL the
                                                // placeholder, so the call ended
                                                // up as `dict.__setitem__(None,
                                                // k, v)` instead of `(obj, k,
                                                // v)` — an instant panic
                                                // (`borrow_mut` on the
                                                // placeholder `PyObjectRef::None`,
                                                // which isn't `Mut`).
                                                PyObject::BuiltinMethod {
                                                    name: m_name,
                                                    func,
                                                    ..
                                                } => {
                                                    found = Some(PyObjectRef::imm(
                                                        PyObject::BuiltinMethod {
                                                            name: m_name.clone(),
                                                            func: *func,
                                                            self_obj: obj.clone(),
                                                        },
                                                    ));
                                                    break;
                                                }
                                                PyObject::Property(ref d) if d.getter.is_some() => {
                                                    let g = d.getter.as_ref().unwrap();
                                                    found = Some(
                                                        builtin_call(g, &[obj.clone()])
                                                            .unwrap_or_else(|_| val.clone()),
                                                    );
                                                    break;
                                                }
                                                // Staticmethods (explicit, or
                                                // implicit like `__new__`) are
                                                // never bound to `obj` — unwrap
                                                // directly, matching how plain
                                                // class-attribute access already
                                                // treats StaticMethod.
                                                PyObject::StaticMethod { func } => {
                                                    found = Some(func.clone());
                                                    break;
                                                }
                                                // A `@classmethod`-wrapped method found on an
                                                // ancestor's dict via `super()` (e.g.
                                                // `super().setUpClass()` inside a subclass's own
                                                // `setUpClass` override, real trigger: `unittest`'s
                                                // own `TestCase.setUpClass`/`tearDownClass`) — the
                                                // catch-all arm below returned the raw
                                                // `PyObject::ClassMethod` wrapper UNCHANGED, which
                                                // isn't itself callable (`TypeError: 'classmethod'
                                                // object is not callable`). `obj` here is already
                                                // the class itself in this calling convention (see
                                                // this match's own comment on `obj_type` above, for
                                                // the "obj is a class/type" classmethod-style
                                                // form), so binding is the same shape as the
                                                // `Function`/`BuiltinFunction` case: wrap in a
                                                // `BoundMethod` with `self_obj: obj.clone()`.
                                                PyObject::ClassMethod { func } => {
                                                    found = Some(PyObjectRef::new(
                                                        PyObject::BoundMethod {
                                                            func: func.clone(),
                                                            self_obj: obj.clone(),
                                                        },
                                                    ));
                                                    break;
                                                }
                                                _ => {
                                                    found = Some(val.clone());
                                                    break;
                                                }
                                            }
                                        }
                                    }
                                }
                                if let Some(found) = found {
                                    return Ok(found);
                                }
                            }
                        }
                    }
                }
                // Not found via any Type in the mro: for a class that
                // transparently subclasses list/dict/str, `super().append(x)`
                // etc. must still reach the native backing (list/dict/str
                // themselves aren't PyObject::Type, so they're invisible to
                // the mro walk above).
                if name == "__init__" {
                    if let Some(kind) = native_base_of_type(&{
                        if let PyObject::Instance { typ, .. } = &*obj.borrow() {
                            typ.clone()
                        } else {
                            return Err(PyError::attribute_error(
                                "'super' object has no attribute '__init__'".to_string(),
                            ));
                        }
                    }) {
                        let target = obj.clone();
                        return Ok(PyObjectRef::new(PyObject::Closure(Rc::new(
                            move |args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                                let native = synthesize_native_init(&kind, args, &[])?;
                                if let PyObject::Instance { dict, .. } = &mut *target.borrow_mut() {
                                    dict.insert(NATIVE_BACKING_KEY.to_string(), native);
                                }
                                Ok(py_none())
                            },
                        ))));
                    }
                }
                // `super().__setattr__(name, value)`/`__delattr__(name)` —
                // the real `object.__setattr__`/`__delattr__` (a plain
                // generic attribute set/delete) isn't exposed as a gettable
                // attribute anywhere either (same class of gap as
                // `__init__` just above), needed by real code that
                // deliberately bypasses an overridden `__setattr__` this
                // way (a frozen-dataclass-style pattern — real trigger:
                // CPython 3.14's own `Lib/_colorize.py`'s
                // `ThemeSection.__post_init__`).
                if name == "__setattr__" || name == "__delattr__" {
                    let target = obj.clone();
                    let is_delete = name == "__delattr__";
                    return Ok(PyObjectRef::new(PyObject::Closure(Rc::new(
                        move |args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                            if args.is_empty() {
                                return Err(PyError::type_error("missing required argument: name"));
                            }
                            let attr_name = args[0].str();
                            if is_delete {
                                target.borrow_mut().del_attribute(&attr_name)?;
                            } else {
                                if args.len() < 2 {
                                    return Err(PyError::type_error(
                                        "__setattr__() takes exactly 2 arguments",
                                    ));
                                }
                                target
                                    .borrow_mut()
                                    .set_attribute(&attr_name, args[1].clone())?;
                            }
                            Ok(py_none())
                        },
                    ))));
                }
                // Same story for the operator-level dunders — list/dict
                // don't expose __setitem__/__getitem__/etc. as a plain
                // get_attribute entry either (subscripting/len/iteration go
                // through their own opcode-level dispatch functions
                // instead), so synthesize a callable that invokes those
                // functions directly against the real native backing.
                if let Some(native) = native_backing_of(obj) {
                    let target = native.clone();
                    match name {
                        "__setitem__" => {
                            return Ok(PyObjectRef::new(PyObject::Closure(Rc::new(
                                move |args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                                    if args.len() < 2 {
                                        return Err(PyError::type_error(
                                            "__setitem__() takes exactly 2 arguments",
                                        ));
                                    }
                                    py_setitem(&target, &args[0], args[1].clone())?;
                                    Ok(py_none())
                                },
                            ))));
                        }
                        "__getitem__" => {
                            return Ok(PyObjectRef::new(PyObject::Closure(Rc::new(
                                move |args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                                    if args.is_empty() {
                                        return Err(PyError::type_error(
                                            "__getitem__() takes exactly 1 argument",
                                        ));
                                    }
                                    py_getitem(&target, &args[0])
                                },
                            ))));
                        }
                        "__delitem__" => {
                            return Ok(PyObjectRef::new(PyObject::Closure(Rc::new(
                                move |args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                                    if args.is_empty() {
                                        return Err(PyError::type_error(
                                            "__delitem__() takes exactly 1 argument",
                                        ));
                                    }
                                    py_delitem(&target, &args[0])?;
                                    Ok(py_none())
                                },
                            ))));
                        }
                        "__contains__" => {
                            return Ok(PyObjectRef::new(PyObject::Closure(Rc::new(
                                move |args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                                    if args.is_empty() {
                                        return Err(PyError::type_error(
                                            "__contains__() takes exactly 1 argument",
                                        ));
                                    }
                                    Ok(py_bool(contains_op(&target, &args[0])?))
                                },
                            ))));
                        }
                        "__len__" => {
                            return Ok(PyObjectRef::new(PyObject::Closure(Rc::new(
                                move |_args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                                    builtin_len(&[target.clone()])
                                },
                            ))));
                        }
                        "__iter__" => {
                            return Ok(PyObjectRef::new(PyObject::Closure(Rc::new(
                                move |_args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                                    builtin_iter(&[target.clone()])
                                },
                            ))));
                        }
                        _ => {}
                    }
                }
                if let Some(native) = native_backing_of(obj) {
                    if let Ok(val) = native.borrow().get_attribute(&name) {
                        let rebound =
                            if let PyObject::BuiltinMethod { name: n, func, .. } = &*val.borrow() {
                                PyObjectRef::imm(PyObject::BuiltinMethod {
                                    name: n.clone(),
                                    func: *func,
                                    self_obj: native.clone(),
                                })
                            } else {
                                val.clone()
                            };
                        return Ok(rebound);
                    }
                }
                Err(PyError::attribute_error(format!(
                    "'super' object has no attribute '{}'",
                    name
                )))
            }
        _ => unreachable!("{} called with non-{} object", stringify!(get), o.type_name()),
    }
}
