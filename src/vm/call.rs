use crate::interner::{self, StrId};
use crate::object::*;
use crate::vm::VirtualMachine;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

impl VirtualMachine {
    pub(crate) fn try_handle_special_builtin(
        &mut self,
        callable: &PyObjectRef,
        args: &[PyObjectRef],
        keywords: &[(String, PyObjectRef)],
    ) -> PyResult<Option<PyObjectRef>> {
        // Fast reject: every special case below only ever fires for a
        // `BuiltinFunction` or `BuiltinMethod` callable matched against a
        // specific known function pointer (`fn_addr_eq`) — never for a
        // plain user-defined `Function`/`Instance`/etc. Checking that ONCE
        // here, instead of letting every one of the ~24 `matches!(...)`
        // probes below run unconditionally, turns 24 separate
        // `RefCell::borrow()` + enum-match checks per call into 1 for the
        // common case (calling an ordinary Python `def`).
        if !matches!(
            &*callable.borrow(),
            PyObject::BuiltinFunction { .. } | PyObject::BuiltinMethod { .. }
        ) {
            return Ok(None);
        }
        {
            let is_type_new = matches!(&*callable.borrow(), PyObject::BuiltinFunction { func, .. } if std::ptr::fn_addr_eq(*func, crate::object::type_new_builtin as crate::object::BuiltinFunc));
            if is_type_new {
                let mut all_args = args.to_vec();
                if !keywords.is_empty() {
                    let mut dict = crate::object::PyDict::new();
                    for (k, v) in keywords {
                        let _ = dict.set(crate::object::py_str(k), v.clone());
                    }
                    all_args.push(PyObjectRef::new(PyObject::Dict(Box::new(dict))));
                }
                return self.type_new_impl(&all_args).map(Some);
            }
        }

        // `getattr(obj, name[, default])` on a plain `Instance` needs to
        // fall back to the type's `__getattr__` (mro-walked) when the raw
        // lookup fails — the same fallback `LOAD_ATTR`'s own opcode
        // handler already does, but `object::builtin_getattr` (a plain
        // `fn(&[PyObjectRef])`, no VM access) can't call a found
        // `__getattr__` itself. Special-cased here (matching `type.__new__`
        // just above) so it happens through the one real, live `self`
        // instead of `with_vm_mut`/`call_bound_method`'s disposable-VM
        // path — a `__getattr__` doing a lazy import (a real, common
        // pattern to dodge circular imports, same as elsewhere this
        // session) would otherwise silently re-import everything from
        // scratch in an empty module registry. Confirmed general, not
        // Django-specific: any two-level `__getattr__` proxy chain where
        // the outer level's own `__getattr__` calls the builtin
        // `getattr(self._wrapped, name)` (real code: Django's
        // `LazySettings`/`UserSettingsHolder`) hit this — `django.conf.
        // settings.LOGGING_CONFIG` (and every other setting not
        // explicitly passed to `settings.configure()`) failed with a
        // nonsensical "instance has no attribute" instead of falling
        // through to the wrapped default-settings module.
        {
            let is_getattr = matches!(&*callable.borrow(), PyObject::BuiltinFunction { func, .. } if std::ptr::fn_addr_eq(*func, crate::object::builtin_getattr as crate::object::BuiltinFunc));
            if is_getattr && (args.len() == 2 || args.len() == 3) {
                let obj = args[0].clone();
                // The attribute NAME must be a `str` (or subclass) — a
                // non-string is CPython's `TypeError: attribute name must
                // be string, not 'int'`, never an AttributeError about the
                // stringified name (test_builtin's `test_getattr`).
                crate::object::check_attr_name_is_str(&args[1])?;
                let attr_name = args[1].str();
                // `re.Pattern.__copy__`/`__deepcopy__` return self
                // (identity for `copy.copy`/`copy.deepcopy`; test_re:
                // test_copying). Native methods don't bind, so capture
                // the instance ref in a closure here.
                if (attr_name == "__copy__" || attr_name == "__deepcopy__")
                    && matches!(&*obj.borrow(), PyObject::CompiledRegex { .. })
                {
                    let self_clone = obj.clone();
                    return Ok(Some(PyObjectRef::new(PyObject::Closure(std::rc::Rc::new(
                        move |_| Ok(self_clone.clone()),
                    )))));
                }
                if std::env::var("RPY_DEBUG_GETATTR").is_ok() {
                    let type_name = if let PyObject::Instance { typ, .. } = &*obj.borrow() {
                        if let PyObject::Type { name, .. } = &*typ.borrow() {
                            name.clone()
                        } else {
                            "?".to_string()
                        }
                    } else {
                        obj.borrow().type_name().to_string()
                    };
                    eprintln!("GETATTR: obj_type={} attr={}", type_name, attr_name);
                }
                // Instance's own __dict__ wins over any class-level
                // descriptor (non-data-descriptor precedence). Only past
                // that do we need real descriptor-protocol dispatch —
                // `object::builtin_getattr`'s plain `get_attribute` (the
                // "direct" fallback below) returns raw, un-invoked
                // `property`/custom-`__get__` values otherwise, unlike
                // LOAD_ATTR's own opcode handler. Confirmed general via a
                // Django-free repro: `getattr(obj, 'some_property')`
                // returned the `property` object itself instead of calling
                // its getter, and a custom descriptor's `__get__` was
                // never invoked at all.
                let own_dict_hit = if let PyObject::Instance { dict, .. } = &*obj.borrow() {
                    dict.get(&attr_name).cloned()
                } else {
                    None
                };
                if let Some(v) = own_dict_hit {
                    return Ok(Some(v));
                }
                if let Some(v) = self.resolve_descriptor_attr(&obj, &attr_name) {
                    return Ok(Some(v));
                }
                // `resolve_descriptor_attr` returning `None` is ambiguous —
                // "no such descriptor here" (fall through to the plain
                // lookup below) vs. "found one, but invoking it raised" (see
                // its own doc comment) — check for the latter and propagate
                // it for real instead of silently falling back to the
                // "direct" (non-invoking) lookup, which would otherwise
                // return the un-invoked descriptor as if it were a normal
                // attribute value.
                if let Some(e) = crate::vm::descriptor::take_pending_descriptor_error() {
                    return Err(e);
                }
                let direct = obj.borrow().get_attribute(&attr_name);
                match direct {
                    Ok(v) => {
                        // object.rs's plain get_attribute (used for the
                        // "direct" success path here) doesn't auto-bind a
                        // plain Function found on an Instance into a
                        // BoundMethod — only the LOAD_ATTR opcode's own,
                        // separate logic does that. Without this,
                        // `getattr(instance, name)` for a real method
                        // returns it UNBOUND while `instance.name` (real
                        // attribute syntax) correctly binds it — an
                        // inconsistency that silently drops `self` the
                        // moment calling code relies on `getattr()` instead
                        // of dot access (a common proxy-object idiom, e.g.
                        // `new_method_proxy`-style `__getattr__` forwarding
                        // via `getattr(self._wrapped, name)`).
                        let is_instance_obj = matches!(&*obj.borrow(), PyObject::Instance { .. });
                        let is_function = matches!(&*v.borrow(), PyObject::Function(_));
                        if is_instance_obj && is_function {
                            return Ok(Some(PyObjectRef::imm(PyObject::BoundMethod {
                                func: v,
                                self_obj: obj.clone(),
                            })));
                        }
                        // `@classmethod`-decorated attributes found on a
                        // class (`obj` a `PyObject::Type`) come back from
                        // plain `get_attribute` as the raw, un-invoked
                        // `ClassMethod` descriptor — only LOAD_ATTR's own
                        // opcode handler binds it into a callable
                        // `BoundMethod`. Without this, `getattr(SomeClass,
                        // 'a_classmethod')()` raised `TypeError:
                        // 'classmethod' object is not callable` even
                        // though `SomeClass.a_classmethod()` worked fine.
                        // Real trigger: `unittest.suite.py`'s
                        // `getattr(currentClass, 'setUpClass', None)` —
                        // every `TestCase` subclass's default
                        // `@classmethod setUpClass`/`tearDownClass` hit
                        // this the moment `_isnotsuite()` (itself only
                        // fixed to work correctly this same session) let
                        // per-class fixture handling actually run for the
                        // first time.
                        let is_type_obj = matches!(&*obj.borrow(), PyObject::Type { .. });
                        if is_type_obj {
                            if let PyObject::ClassMethod { func } = &*v.borrow() {
                                return Ok(Some(PyObjectRef::imm(PyObject::BoundMethod {
                                    func: func.clone(),
                                    self_obj: obj.clone(),
                                })));
                            }
                        }
                        // Native (non-Instance) types — File, List, Dict,
                        // Set, ... — expose their own methods as
                        // `BuiltinMethod` values with a `PyObject::None`
                        // PLACEHOLDER `self_obj`, meant to always be rebound
                        // to whatever object they were actually looked up
                        // on (LOAD_ATTR's own opcode handling does this
                        // rebinding inline; plain `get_attribute` — used for
                        // this "direct" success path — never did). Without
                        // this, `getattr(some_file, 'write')` (a real,
                        // common proxy idiom — e.g. `unittest`'s own
                        // `_WritelnDecorator.__getattr__` forwarding via
                        // `getattr(self.stream, attr)`) returned a `write`
                        // method still bound to that placeholder `None`,
                        // so calling it failed with "write on non-file".
                        let rebind_builtin_method = if let PyObject::BuiltinMethod {
                            name,
                            func,
                            self_obj,
                        } = &*v.borrow()
                        {
                            let placeholder = matches!(&*self_obj.borrow(), PyObject::None);
                            if placeholder && !matches!(&*obj.borrow(), PyObject::Instance { .. }) {
                                Some((name.clone(), *func))
                            } else {
                                None
                            }
                        } else {
                            None
                        };
                        if let Some((name, func)) = rebind_builtin_method {
                            return Ok(Some(PyObjectRef::imm(PyObject::BuiltinMethod {
                                name,
                                func,
                                self_obj: obj.clone(),
                            })));
                        }
                        return Ok(Some(v));
                    }
                    Err(direct_err) => {
                        let getattr_fn = if let PyObject::Instance { typ, .. } = &*obj.borrow() {
                            crate::object::lookup_dunder_via_mro(typ, "__getattr__")
                        } else {
                            None
                        };
                        if let Some(f) = getattr_fn {
                            // Snapshot `sys.exc_info()` before calling the
                            // Python-level `__getattr__` fallback so it can be
                            // restored when its exception is swallowed below
                            // (the `args.len() == 3` default-value case) — a
                            // `raise AttributeError(...)` inside `__getattr__`
                            // with no enclosing `try/except` sets
                            // `exc_type`/`exc_value` (via `RAISE_VARARGS`) and
                            // propagates out of `call_function` as a plain
                            // `Err` with no `POP_EXCEPT` ever running to
                            // restore them. Left uncorrected here,
                            // `getattr(obj, name, default)` — which real
                            // Python never lets affect `sys.exc_info()` at
                            // all — permanently corrupted it for any caller
                            // still inside its own `except:` block (real
                            // trigger: `unittest.mock`'s `NonCallableMock.
                            // __getattr__`, whose spec-checking machinery
                            // uses exactly this raise-and-discard idiom
                            // internally).
                            let saved_exc = (self.exc_type.clone(), self.exc_value.clone());
                            match self.call_function(
                                f,
                                vec![obj.clone(), crate::object::py_str(&attr_name)],
                                vec![],
                            ) {
                                Ok(v) => return Ok(Some(v)),
                                Err(_) if args.len() == 3 => {
                                    self.exc_type = saved_exc.0;
                                    self.exc_value = saved_exc.1;
                                    return Ok(Some(args[2].clone()));
                                }
                                Err(e) => return Err(e),
                            }
                        }
                        if args.len() == 3 {
                            return Ok(Some(args[2].clone()));
                        }
                        return Err(direct_err);
                    }
                }
            }
        }

        // `hasattr(obj, name)` — same descriptor-protocol gap as `getattr`
        // just above (`object::builtin_hasattr`, also a plain `fn(&[PyObjectRef])`
        // with no VM access, can only do raw `get_attribute`): a `property`/
        // custom descriptor whose getter RAISES should make `hasattr` return
        // False (matching real Python's "hasattr calls getattr and catches
        // the exception" semantics), but raw retrieval never invokes the
        // getter at all, so it can never observe that failure.
        // Also, Instances with `__getattr__` (notably `unittest.mock`'s
        // `MagicMock`, whose `__getattr__` auto-creates child mocks for any
        // attribute) must have `hasattr(mock, "any_name")` return True, since
        // `getattr(mock, "any_name")`/dot access both succeed via `__getattr__`.
        // The plain `get_attribute` path never consults `__getattr__`, so
        // `hasattr` was always False for such mocks (real trigger:
        // `_colorize.can_colorize`'s `hasattr(file, "fileno")` guard, which
        // incorrectly returned False for a mocked `sys.stdout`/`file`).
        {
            let is_hasattr = matches!(&*callable.borrow(), PyObject::BuiltinFunction { func, .. } if std::ptr::fn_addr_eq(*func, crate::object::builtin_hasattr as crate::object::BuiltinFunc));
            if is_hasattr && args.len() == 2 {
                let obj = args[0].clone();
                crate::object::check_attr_name_is_str(&args[1])?;
                let attr_name = args[1].str();
                let own_dict_hit = if let PyObject::Instance { dict, .. } = &*obj.borrow() {
                    dict.get(&attr_name).cloned()
                } else {
                    None
                };
                if own_dict_hit.is_some() {
                    return Ok(Some(py_bool(true)));
                }
                if self.resolve_descriptor_attr(&obj, &attr_name).is_some() {
                    return Ok(Some(py_bool(true)));
                }
                // Real `hasattr()` (Python 3+) only masks `AttributeError` —
                // any other exception raised by a descriptor (a property
                // getter, say) propagates out of `hasattr()` itself instead
                // of just making it return `False`. `PyError::type_name()`
                // is NOT the right check here: a user-level `raise
                // AttributeError(...)` is wrapped as `PyError::Exception`
                // (whose OWN `.type_name()` is the fixed literal string
                // "Exception", not the real exception's name — only an
                // internally-constructed `PyError::AttributeError` variant
                // reports that directly) — use the same `is_attribute_error`
                // helper this batch's `introspection1.rs`/`introspection2.rs`
                // fixes already use for exactly this reason, which unwraps
                // `PyError::Exception`'s real wrapped exception object.
                if let Some(e) = crate::vm::descriptor::take_pending_descriptor_error() {
                    if !crate::object::is_attribute_error(&e) {
                        return Err(e);
                    }
                    // A descriptor WAS found and invoking it raised
                    // AttributeError — real `hasattr()` is False here, full
                    // stop. Falling through to the plain `get_attribute`
                    // check below would be wrong: that's a NON-invoking
                    // lookup, so it would find the raw, un-invoked
                    // `property` object still sitting in the class's dict
                    // and wrongly report `True` (attribute *access* failed;
                    // the descriptor's mere *existence* is not what
                    // `hasattr` means).
                    return Ok(Some(py_bool(false)));
                }
                if obj.borrow().get_attribute(&attr_name).is_ok() {
                    return Ok(Some(py_bool(true)));
                }
                // Fall back to `__getattr__` (e.g. MagicMock's auto-creation)
                // — `hasattr` is `getattr` catching failures, so if `__getattr__`
                // succeeds, hasattr is True; if it raises, hasattr is False.
                if let PyObject::Instance { typ, .. } = &*obj.borrow() {
                    if let Some(f) = crate::object::lookup_dunder_via_mro(typ, "__getattr__") {
                        // Same `sys.exc_info()` corruption risk as the
                        // `getattr()` fallback above: `hasattr()` always
                        // swallows whatever `__getattr__` raises (that's the
                        // whole point — a raise means "no such attribute",
                        // hence `False`), and real Python never lets that
                        // leak into `sys.exc_info()`.
                        let saved_exc = (self.exc_type.clone(), self.exc_value.clone());
                        let res = self.call_function(
                            f,
                            vec![obj.clone(), crate::object::py_str(&attr_name)],
                            vec![],
                        );
                        // Only an AttributeError means "no such attribute"
                        // (=> `False`); every other exception — a
                        // `__getattr__` raising ValueError/SystemExit, say —
                        // propagates out of `hasattr()` itself (CPython 3+,
                        // test_builtin's `test_hasattr`).
                        let out = match res {
                            Ok(_) => Ok(Some(py_bool(true))),
                            Err(e) if crate::object::is_attribute_error(&e) => {
                                Ok(Some(py_bool(false)))
                            }
                            Err(e) => Err(e),
                        };
                        self.exc_type = saved_exc.0;
                        self.exc_value = saved_exc.1;
                        return out;
                    }
                }
                return Ok(Some(py_bool(false)));
            }
        }

        // `sys.exc_info()` — same `with_vm_mut`-is-unconditional-UB class
        // of bug as the `exec()`/`eval()` fix just below (confirmed via the
        // simplest possible repro: `except Exception: sys.exc_info()`
        // reliably segfaulting). Read the real, live VM's own exception
        // fields directly instead.
        {
            let is_exc_info = matches!(&*callable.borrow(), PyObject::BuiltinFunction { func, .. } if std::ptr::fn_addr_eq(*func, crate::modules::sys_exc_info_builtin as crate::object::BuiltinFunc));
            if is_exc_info {
                if std::env::var("RPY_DEBUG_EXCINFO").is_ok() {
                    eprintln!(
                        "READ exc_info: type={:?} value={:?}",
                        self.exc_type.as_ref().map(|v| v.repr()),
                        self.exc_value.as_ref().map(|v| v.repr())
                    );
                }
                // The real __traceback__ chain lives on the exception object
                // itself (`exc_traceback` is only ever set to None at raise
                // time) — `sys.__excepthook__(*sys.exc_info())` needs it.
                let tb = self
                    .exc_value
                    .as_ref()
                    .and_then(|v| {
                        v.borrow()
                            .get_attribute("__traceback__")
                            .ok()
                            .filter(|t| !matches!(&*t.borrow(), PyObject::None))
                    })
                    .unwrap_or_else(py_none);
                return Ok(Some(py_tuple(vec![
                    self.exc_type.clone().unwrap_or_else(py_none),
                    self.exc_value.clone().unwrap_or_else(py_none),
                    tb,
                ])));
            }
        }

        // `sys.exception()` (3.11+) — same fix, same reason, as
        // `sys.exc_info()` just above: reads `self.exc_value` directly
        // instead of going through `with_vm_mut`, which gave the wrong
        // (always-empty) result from this reentrant calling context.
        {
            let is_exception = matches!(&*callable.borrow(), PyObject::BuiltinFunction { func, .. } if std::ptr::fn_addr_eq(*func, crate::modules::sys_exception_builtin as crate::object::BuiltinFunc));
            if is_exception {
                return Ok(Some(self.exc_value.clone().unwrap_or_else(py_none)));
            }
        }

        // `sys.excepthook`/`sys.__excepthook__` — build the report with
        // `build_excepthook_report` (VM-independent) and write it to the
        // CURRENT `sys.stderr` via `self`. Doing the write through
        // `with_vm_mut` from inside this live call chain segfaults (the
        // thread-local VM pointer is stale here), and Rust's own stderr
        // bypasses `support.captured_stderr`'s Python-level redirect.
        {
            let is_excepthook = matches!(&*callable.borrow(), PyObject::BuiltinFunction { func, .. } if std::ptr::fn_addr_eq(*func, crate::modules::sys_excepthook_builtin as crate::object::BuiltinFunc));
            if is_excepthook {
                let report = crate::modules::build_excepthook_report(&args)?;
                let stderr = self
                    .modules
                    .get("sys")
                    .and_then(|m| {
                        if let PyObject::Module { dict, .. } = &*m.borrow() {
                            dict.get_str("stderr").cloned()
                        } else {
                            None
                        }
                    })
                    .unwrap_or_else(py_none);
                if !matches!(&*stderr.borrow(), PyObject::None) {
                    let _ = crate::object::call_method_rebound(
                        self,
                        &stderr,
                        "write",
                        vec![py_str(&report)],
                    );
                }
                return Ok(Some(py_none()));
            }
        }

        // `list_iterator.__reduce__()` — CPython resolves the `iter`
        // callable through `_PyEval_GetBuiltin` at call time
        // (Objects/iterobject.c `listiter_reduce`), so a frame whose
        // `__builtins__` is a custom mapping controls it: a mapping
        // without `iter` must fail with an AttributeError naming it, and
        // one with `iter` gets that object back (test_builtin's
        // `test_eval_builtins_mapping_reduce`).
        {
            let is_list_iter_reduce = matches!(&*callable.borrow(), PyObject::BuiltinMethod { func, .. } if std::ptr::fn_addr_eq(*func, crate::object::list_iter_reduce as crate::object::BuiltinFunc));
            if is_list_iter_reduce {
                let frame = self
                    .frames
                    .last()
                    .ok_or_else(|| PyError::runtime_error("no frame"))?;
                let builtins = crate::vm::helpers::frame_exec_builtins(frame)?;
                if let Some(bm) = builtins {
                    let iter_fn = crate::vm::helpers::mapping_builtin_get(&bm, "iter")?
                        .ok_or_else(|| {
                            PyError::attribute_error("'iter' not found in __builtins__")
                        })?;
                    // A `BuiltinMethod` receives its `self` later in
                    // `call_function` (the interception runs before the
                    // placeholder `self_obj` is substituted), so the
                    // iterator itself is the callable's bound `self_obj`.
                    let self_obj = match &*callable.borrow() {
                        PyObject::BuiltinMethod { self_obj, .. } => self_obj.clone(),
                        _ => args.first().cloned().unwrap_or_else(py_none),
                    };
                    let (list, index) = match &*self_obj.borrow() {
                        PyObject::ListIter { list, index } => (list.clone(), *index),
                        _ => return Err(PyError::runtime_error("__reduce__ on non-list_iterator")),
                    };
                    return Ok(Some(py_tuple(vec![
                        iter_fn,
                        py_tuple(vec![py_list(list)]),
                        py_int(index as i64),
                    ])));
                }
            }
        }

        // `sys.breakpointhook`/`sys.__breakpointhook__` and the
        // `breakpoint()` builtin that calls it — both need the live VM to
        // resolve `$PYTHONBREAKPOINT` (an import) and then call the
        // resolved hook. `breakpoint()` itself only reads the CURRENT
        // `sys.breakpointhook` out of the live sys module dict: a test (or
        // user) may have replaced it with an arbitrary callable, which then
        // receives `breakpoint()`'s own args/kwargs verbatim; a deleted
        // attribute is CPython's `RuntimeError: lost sys.breakpointhook`
        // (`_PySys_GetRequiredAttrString` in `Python/sysmodule.c`).
        {
            let is_bph = matches!(&*callable.borrow(), PyObject::BuiltinFunction { func, .. } if std::ptr::fn_addr_eq(*func, crate::modules::sys_breakpointhook_builtin as crate::object::BuiltinFunc));
            let is_breakpoint = matches!(&*callable.borrow(), PyObject::BuiltinFunction { func, .. } if std::ptr::fn_addr_eq(*func, crate::object::builtin_breakpoint as crate::object::BuiltinFunc));
            if is_bph {
                return crate::modules::breakpointhook_resolve_and_call(self, args, keywords)
                    .map(Some);
            }
            if is_breakpoint {
                let hook = self
                    .modules
                    .get("sys")
                    .and_then(|m| {
                        if let PyObject::Module { dict, .. } = &*m.borrow() {
                            dict.get_str("breakpointhook").cloned()
                        } else {
                            None
                        }
                    })
                    .ok_or_else(|| PyError::runtime_error("lost sys.breakpointhook"))?;
                let still_default = matches!(&*hook.borrow(), PyObject::BuiltinFunction { func, .. } if std::ptr::fn_addr_eq(*func, crate::modules::sys_breakpointhook_builtin as crate::object::BuiltinFunc));
                if still_default {
                    return crate::modules::breakpointhook_resolve_and_call(self, args, keywords)
                        .map(Some);
                }
                return self
                    .call_function(hook, args.to_vec(), keywords.to_vec())
                    .map(Some);
            }
        }

        // `sys.getrecursionlimit()`/`setrecursionlimit()` — read/write
        // `self.recursion_limit` directly (same `with_vm_mut`-avoidance
        // pattern as everything else here) instead of the fallback
        // `with_vm_mut`-based native fns, which are otherwise unconditional
        // UB from within a live call chain like every other case on this
        // page.
        {
            let is_getrecursionlimit = matches!(&*callable.borrow(), PyObject::BuiltinFunction { func, .. } if std::ptr::fn_addr_eq(*func, crate::modules::sys_getrecursionlimit_builtin as crate::object::BuiltinFunc));
            if is_getrecursionlimit {
                return Ok(Some(py_int(self.recursion_limit as i64)));
            }
            let is_setrecursionlimit = matches!(&*callable.borrow(), PyObject::BuiltinFunction { func, .. } if std::ptr::fn_addr_eq(*func, crate::modules::sys_setrecursionlimit_builtin as crate::object::BuiltinFunc));
            if is_setrecursionlimit {
                let n = args.get(0).and_then(|a| a.as_i64()).ok_or_else(|| {
                    PyError::type_error("setrecursionlimit() requires an integer argument")
                })?;
                if n < 1 {
                    return Err(PyError::value_error(
                        "recursion limit must be greater or equal than 1",
                    ));
                }
                self.recursion_limit = n as usize;
                return Ok(Some(py_none()));
            }
        }

        // `print()` — needs the live VM to look up the CURRENT `sys.stdout`
        // (not a cached reference) and to accept `sep`/`end`/`file`/`flush`
        // keyword arguments, which the generic `BuiltinFunction` dispatch
        // path further below would otherwise pack into a trailing dict
        // ARGUMENT (this project's established kwargs-passing convention
        // for plain builtins) — silently printing that dict as if it were
        // one more thing to print, since the old implementation just joined
        // every element of `args` unconditionally. See `print_with_vm`'s
        // own doc comment for the full story.
        if matches!(&*callable.borrow(), PyObject::BuiltinFunction { func, .. } if std::ptr::fn_addr_eq(*func, crate::object::builtin_print as crate::object::BuiltinFunc))
        {
            return crate::object::print_with_vm(self, args, keywords).map(Some);
        }

        // `IOBase.__exit__` (inherited by every native `io` type, and by any
        // Python subclass — e.g. `zipfile._ZipWriteFile` — that doesn't
        // override `__exit__` itself) needs the live VM to invoke
        // `self.close()` (see `iobase_exit_with_vm`'s own doc comment).
        // Attribute lookup (`obj.__exit__`) auto-binds a plain
        // `BuiltinFunction` found via the MRO into a `BuiltinMethod` with
        // `self` already attached — so this must match BOTH the unbound
        // and bound shapes, not just `BuiltinFunction` (mirrors the
        // `context_run_fallback`/`BuiltinMethod` check above).
        {
            let unbound_hit = matches!(&*callable.borrow(), PyObject::BuiltinFunction { func, .. } if std::ptr::fn_addr_eq(*func, crate::modules::iobase_exit_marker as crate::object::BuiltinFunc));
            if unbound_hit {
                return crate::modules::iobase_exit_with_vm(self, args).map(Some);
            }
            if let PyObject::BuiltinMethod { func, self_obj, .. } = &*callable.borrow() {
                if std::ptr::fn_addr_eq(
                    *func,
                    crate::modules::iobase_exit_marker as crate::object::BuiltinFunc,
                ) {
                    let mut all_args = vec![self_obj.clone()];
                    all_args.extend_from_slice(args);
                    return crate::modules::iobase_exit_with_vm(self, &all_args).map(Some);
                }
            }
        }

        // `bytes.decode(b"...")`, `bytes.hex(b)`, ... — the UNBOUND form of
        // the instance methods, which CPython also exposes on the class
        // object and real code uses (every `test_pdb` subprocess helper does
        // `bytes.decode(stdout)`). All of them share ONE marker fn, with
        // the method name carried by the `BuiltinFunction`'s own `name`
        // field, and dispatch to the instance's attribute of that name.
        if let PyObject::BuiltinFunction { func, name } = &*callable.borrow() {
            if std::ptr::fn_addr_eq(
                *func,
                crate::modules::core::builtins::collection_types::bytes_unbound_method_marker
                    as crate::object::BuiltinFunc,
            ) {
                let method = name.clone();
                let mut all_args = args.to_vec();
                if all_args.is_empty() {
                    return Err(PyError::type_error(format!(
                        "{}() missing required argument",
                        method
                    )));
                }
                let receiver = all_args.remove(0);
                let bound = receiver.borrow().get_attribute(&method).map_err(|_| {
                    PyError::type_error(format!(
                        "descriptor '{}' requires a '{}' object",
                        method,
                        receiver.borrow().type_name()
                    ))
                })?;
                // Rebind the method to the receiver the way the LOAD_ATTR
                // path does, then call it with the REMAINING arguments (a
                // `BuiltinMethod` whose `self_obj` is set gets it prepended).
                let rebound = {
                    let borrowed = bound.borrow();
                    match &*borrowed {
                        PyObject::BuiltinMethod { name, func, .. } => {
                            PyObjectRef::imm(PyObject::BuiltinMethod {
                                name: name.clone(),
                                func: *func,
                                self_obj: receiver.clone(),
                            })
                        }
                        _ => bound.clone(),
                    }
                };
                return self.call_function(rebound, all_args, Vec::new()).map(Some);
            }
        }

        // Every `zipimporter` instance method (`is_package`, `get_source`,
        // `find_spec`, `exec_module`, ...) shares ONE marker fn
        // (`zipimporter_method_marker`) rather than one distinct marker
        // per method — real dispatch happens by the `name` field the
        // `BuiltinMethod` already carries (see
        // `zipimporter_method_with_vm`'s doc comment).
        if let PyObject::BuiltinMethod {
            func,
            self_obj,
            name,
        } = &*callable.borrow()
        {
            if std::ptr::fn_addr_eq(
                *func,
                crate::modules::zipimporter_method_marker as crate::object::BuiltinFunc,
            ) {
                let method = name.clone();
                let self_obj = self_obj.clone();
                return crate::modules::zipimporter_method_with_vm(self, &method, &self_obj, args)
                    .map(Some);
            }
        }

        // `TextIOWrapper`'s `read`/`readline`/`write`/`seek`/`tell`/`flush`/
        // `close`/`truncate`/`fileno`/`isatty`/`writelines`/`readlines`/
        // `name`/`mode` all share ONE marker (`tiw_delegate_marker`), same
        // convention as `zipimporter_method_marker` right above — see
        // `tiw_delegate_marker`'s own doc comment for why this needs the
        // live VM instead of `with_vm_mut`. Must handle both the auto-bound
        // `BuiltinMethod` shape (ordinary `f.read()` dot-call syntax) and
        // the bare unbound `BuiltinFunction` shape (a property getter, e.g.
        // `f.name`, is invoked as `getter(self)` directly, never auto-bound
        // — mirrors `iobase_exit_marker`'s own dual-shape handling above).
        if let PyObject::BuiltinMethod {
            func,
            self_obj,
            name,
        } = &*callable.borrow()
        {
            if std::ptr::fn_addr_eq(
                *func,
                crate::modules::tiw_delegate_marker as crate::object::BuiltinFunc,
            ) {
                let method = name.clone();
                let mut all_args = vec![self_obj.clone()];
                all_args.extend_from_slice(args);
                return crate::modules::tiw_delegate_with_vm(self, &method, &all_args).map(Some);
            }
        }
        if let PyObject::BuiltinFunction { func, name } = &*callable.borrow() {
            if std::ptr::fn_addr_eq(
                *func,
                crate::modules::tiw_delegate_marker as crate::object::BuiltinFunc,
            ) {
                let method = name.clone();
                return crate::modules::tiw_delegate_with_vm(self, &method, args).map(Some);
            }
        }

        // `contextvars.Context.run(callable, *args, **kwargs)` — needs the
        // live VM to call `callable` with its OWN real positional/keyword
        // arguments forwarded, not this project's usual "pack kwargs into a
        // trailing dict ARGUMENT" convention (which is ambiguous here: a
        // genuinely positional dict passed to the wrapped callable would be
        // indistinguishable from keyword arguments meant for it). Bound as
        // `PyObject::BuiltinMethod` (found via `ctx.run`'s attribute
        // lookup), unlike `print`'s bare-function case just above.
        if let PyObject::BuiltinMethod { func, self_obj, .. } = &*callable.borrow() {
            if std::ptr::fn_addr_eq(
                *func,
                crate::modules::context_run_fallback as crate::object::BuiltinFunc,
            ) {
                let self_obj = self_obj.clone();
                return crate::modules::context_run_with_vm(self, self_obj, args, keywords)
                    .map(Some);
            }
        }

        // (Bare `eval`/`exec` caller-namespace handling lives in the
        // `exec(source[, globals[, locals]])` interception further below,
        // which already snapshots caller fast-locals for argument-less
        // calls — see the bare-`exec` namespace branch there.)

        // `globals()`/`locals()` — same `with_vm_mut`-is-unconditional-UB
        // class of bug (confirmed via a general repro: `def f(): locals()`
        // — not a segfault this time, but `vm.frames` reading back empty
        // through the aliased pointer, "RuntimeError: no frame", even
        // though the real VM's frame stack plainly wasn't empty). Read
        // `self.frames` directly instead of going through `with_vm_mut`.
        {
            let is_globals = matches!(&*callable.borrow(), PyObject::BuiltinFunction { func, .. } if std::ptr::fn_addr_eq(*func, crate::object::builtin_globals as crate::object::BuiltinFunc));
            let is_locals = matches!(&*callable.borrow(), PyObject::BuiltinFunction { func, .. } if std::ptr::fn_addr_eq(*func, crate::object::builtin_locals as crate::object::BuiltinFunc));
            if is_globals || is_locals {
                let frame = self
                    .frames
                    .last()
                    .ok_or_else(|| PyError::runtime_error("no frame"))?;
                // An `exec`/`eval` frame with an explicit globals MAPPING
                // has f_globals BEING that object, so `globals()` must
                // return it itself (`eval("globals()['x']", globals=data)`
                // reads `data`, and `globals() is data` — test_builtin's
                // `test_eval_kwargs`/`test_general_eval`).
                if let Some(g) = frame.exec_globals.clone() {
                    if is_globals {
                        return Ok(Some(g));
                    }
                    // Likewise `locals()`: a distinct locals mapping is
                    // returned as-is.
                    if let Some(l) = frame.exec_locals.clone() {
                        return Ok(Some(l));
                    }
                }
                if is_globals {
                    // Return a LIVE view of the frame's globals (same backing
                    // `Rc<RefCell<HashMap>>` that LOAD_GLOBAL reads) so
                    // mutations like `globals()['len'] = f` are visible to
                    // name resolution — matching real CPython, where
                    // `globals()` IS the module dict, not a copy
                    // (test_dynamic::test_globals_shadow_builtins).
                    return Ok(Some(PyObjectRef::new(PyObject::Globals(
                        frame.globals.clone(),
                    ))));
                }
                // `locals()` for an `exec(code, g, l)` frame: the distinct
                // mapping itself, not a merged copy of the frame.
                if let Some(l) = frame.exec_locals.clone() {
                    return Ok(Some(l));
                }
                let mut d = crate::object::PyDict::new();
                // Merge fast-locals (function-scope named params/vars,
                // keyed by position against `code.varnames`) with the
                // name-keyed `locals` map (module/class-scope variables,
                // which never go through STORE_FAST at all) — a real
                // snapshot needs both; the pre-fix version only ever
                // read the latter, so a function's own locals() always
                // came back empty regardless of the frame lookup bug.
                for (i, slot) in frame.fast_locals.iter().enumerate() {
                    if let Some(v) = slot {
                        if let Some(&name) = frame.code.varnames.get(i) {
                            d.set(py_str(crate::interner::lookup_str(name)), v.clone())?;
                        }
                    }
                }
                for (k, v) in frame.locals.iter() {
                    let name = crate::interner::lookup(k);
                    d.set(py_str(&name), v.clone())?;
                }
                return Ok(Some(PyObjectRef::new(PyObject::Dict(Box::new(d)))));
            }
        }

        // `sys._getframe(depth=0)` — was a complete no-op stub, always
        // returning `None` regardless of `depth` (`object::core.rs`'s
        // version has no VM access at all to do otherwise). Real trigger:
        // `Lib/test/support/warnings_helper.py`'s `_filterwarnings`
        // (`sys._getframe(2)`, to clear the CALLING module's
        // `__warningregistry__` so warnings can be re-raised) — used by
        // `check_warnings`, itself used pervasively across the corpus by
        // any test asserting on warning behavior. Same `with_vm_mut`-
        // avoidance pattern as `globals()`/`locals()` just above: reads
        // `self.frames` directly. Returns a minimal but real `frame`-shaped
        // `Instance` exposing `f_globals` as a live dict snapshot (each
        // VALUE is the same shared `PyObjectRef` as the frame's real
        // globals entry, so mutating a nested container — e.g. clearing
        // `__warningregistry__` — still affects the real frame, even
        // though the snapshot dict itself is a fresh copy) — enough for
        // this and similar introspection uses, not a full frame object.
        {
            let is_getframe = matches!(&*callable.borrow(), PyObject::BuiltinFunction { func, .. } if std::ptr::fn_addr_eq(*func, crate::modules::sys_getframe_builtin as crate::object::BuiltinFunc));
            if is_getframe {
                let depth = args.first().and_then(|a| a.as_i64()).unwrap_or(0);
                if depth < 0 {
                    return Err(PyError::value_error("call stack is not deep enough"));
                }
                // A generator/coroutine resumed via generator_next_fallback
                // runs its frame in a DISPOSABLE VM whose `frames` stack has
                // ONLY that one frame — so `sys._getframe(2)` inside it
                // (real trigger: warnings_helper._filterwarnings, a
                // generator calling `sys._getframe(2)` to clear the calling
                // module's `__warningregistry__`) raised "call stack is not
                // deep enough". Real CPython's generator frames chain to the
                // CALLER's frames; this VM's don't. Clamp instead of
                // erroring: return the deepest available frame (usually the
                // generator's own), which still gives the caller a usable
                // `f_globals` to operate on.
                let idx = (self.frames.len() as i64) - 1 - depth;
                let frame = if idx >= 0 {
                    self.frames.get(idx as usize)
                } else {
                    self.frames.first()
                };
                let frame =
                    frame.ok_or_else(|| PyError::value_error("call stack is not deep enough"))?;
                // Return the frame's CACHED Python `frame` object (created
                // once, reused) so `sys._getframe()` returns the same object
                // an exception traceback captured as `tb_frame` for this live
                // frame (`tb.tb_frame is sys._getframe()` — CPython's own
                // test_raise asserts exactly this identity).
                let eff_idx = if idx >= 0 { idx as usize } else { 0 };
                if self.frames.get(eff_idx).is_none() {
                    return Err(PyError::value_error("call stack is not deep enough"));
                }
                drop(frame);
                if let Some(fo) = self.frame_object(eff_idx) {
                    return Ok(Some(fo));
                }
                return Err(PyError::value_error("call stack is not deep enough"));
            }
        }

        // `isinstance(obj, cls)`/`issubclass(sub, cls)` — real Python lets a
        // custom METACLASS override these entirely by defining its own
        // `__instancecheck__`/`__subclasscheck__` (distinct from, and rarer
        // than, `__subclasshook__`-based ABC registration, which the
        // generic `builtin_isinstance`/`builtin_issubclass` dispatch
        // already supports elsewhere). Real trigger: CPython's own
        // `test_typechecks.py` (`class ABC(type): def __instancecheck__
        // (cls, inst): ...`). `object::builtin_isinstance`/
        // `builtin_issubclass` are plain `fn(&[PyObjectRef])` with no VM
        // access, so they can never CALL such a hook — only special-cased
        // here, with the real, live `self`, and only when a custom
        // metaclass hook is actually present (checked cheaply up front);
        // falls through to the normal, unmodified dispatch otherwise, so
        // the overwhelmingly common no-custom-metaclass path is completely
        // unaffected. Handles the tuple-of-classes form too (`isinstance
        // (x, (A, B))`) directly here, since `builtin_isinstance`'s OWN
        // internal tuple recursion is a plain Rust call that never reaches
        // this dispatch layer for each member.
        {
            let is_isinstance = matches!(&*callable.borrow(), PyObject::BuiltinFunction { func, .. } if std::ptr::fn_addr_eq(*func, crate::object::builtin_isinstance as crate::object::BuiltinFunc));
            let is_issubclass = matches!(&*callable.borrow(), PyObject::BuiltinFunction { func, .. } if std::ptr::fn_addr_eq(*func, crate::object::builtin_issubclass as crate::object::BuiltinFunc));
            if (is_isinstance || is_issubclass) && args.len() == 2 {
                let hook_name = if is_isinstance {
                    "__instancecheck__"
                } else {
                    "__subclasscheck__"
                };
                let find_hook = |cls: &PyObjectRef| -> Option<PyObjectRef> {
                    if !matches!(&*cls.borrow(), PyObject::Type { .. }) {
                        return None;
                    }
                    let mt = crate::object::metatype_of(cls)?;
                    if std::env::var("RPY_TRACE_IS").is_ok() {
                        eprintln!(
                            "FIND-HOOK cls={} mt={:?} has_hook={}",
                            cls.borrow().type_name(),
                            mt.borrow().type_name(),
                            if let PyObject::Type { dict, .. } = &*mt.borrow() {
                                dict.contains_key_str(hook_name)
                            } else {
                                false
                            }
                        );
                    }
                    let hook = if let PyObject::Type { dict, .. } = &*mt.borrow() {
                        dict.get_str(hook_name).cloned()
                    } else {
                        None
                    };
                    hook
                };
                let classes: Vec<PyObjectRef> = match &*args[1].borrow() {
                    PyObject::Tuple(items) => items.clone(),
                    _ => vec![args[1].clone()],
                };
                if classes.iter().any(|c| find_hook(c).is_some()) {
                    for cls in &classes {
                        if let Some(hook) = find_hook(cls) {
                            let bound = PyObjectRef::imm(PyObject::BoundMethod {
                                func: hook,
                                self_obj: cls.clone(),
                            });
                            let result =
                                self.call_function(bound, vec![args[0].clone()], vec![])?;
                            if result.truthy() {
                                return Ok(Some(py_bool(true)));
                            }
                        } else if is_isinstance {
                            if crate::object::builtin_isinstance(&[args[0].clone(), cls.clone()])?
                                .truthy()
                            {
                                return Ok(Some(py_bool(true)));
                            }
                        } else if crate::object::builtin_issubclass(&[
                            args[0].clone(),
                            cls.clone(),
                        ])?
                        .truthy()
                        {
                            return Ok(Some(py_bool(true)));
                        }
                    }
                    return Ok(Some(py_bool(false)));
                }
            }
        }

        // `__import__(name, ...)` — what every `import` STATEMENT desugars
        // to in real CPython; this interpreter's own `IMPORT_NAME` opcode
        // doesn't route through it, but plenty of real code calls it
        // explicitly (confirmed segfaulting via the simplest possible
        // repro, `__import__("os")` at plain top level — same
        // `with_vm_mut`-is-unconditional-UB class of bug as `exec`/`eval`/
        // `sys.exc_info()`/`globals()`/`locals()` above). Shares
        // `object::import_impl` (extracted out of `builtin_import` for
        // exactly this) with the real VM directly.
        {
            let is_import = matches!(&*callable.borrow(), PyObject::BuiltinFunction { func, .. } if std::ptr::fn_addr_eq(*func, crate::object::builtin_import as crate::object::BuiltinFunc));
            // Real `__import__`'s `name` is commonly passed as a KEYWORD
            // argument too (`__import__(name='sys')` — exactly what
            // `test_builtin.py::BuiltinTest.test_import` exercises). Since
            // `keywords` arrives as a SEPARATE parameter here (not yet
            // packed into `args`), the old `!args.is_empty()` guard was
            // false whenever `name` was keyword-only, silently falling
            // through to `object::builtin_import`'s generic `with_vm_mut`
            // path below — which then treats the whole packed kwargs DICT
            // as the module name (stringifying it to garbage like
            // `"{'name': 'sys'}"`) and feeds that into the import
            // machinery, corrupting `self.modules`'s backing allocation
            // (confirmed via `gdb`: SIGSEGV inside a `HashMap::get("sys")`
            // call in `get_sys_path`, reached via the very same
            // `with_vm_mut` raw-pointer-aliasing UB class documented
            // throughout this function) rather than raising a clean error.
            let name_kw = keywords
                .iter()
                .find(|(k, _)| k == "name")
                .map(|(_, v)| v.clone());
            if is_import && (!args.is_empty() || name_kw.is_some()) {
                // Real CPython rejects `name` given BOTH positionally and by
                // keyword (`__import__('sys', name='sys')`) with a
                // `TypeError` — `test_builtin.py::BuiltinTest.test_import`
                // checks this exact case too.
                if !args.is_empty() && name_kw.is_some() {
                    return Err(PyError::type_error(
                        "argument for __import__() given by name ('name') and position (1)",
                    ));
                }
                let name_obj = args.get(0).cloned().or(name_kw).unwrap();
                // Real `__import__` requires `name` to actually be a `str`
                // (`__import__(1, 2, 3, 4)` — exercised directly by
                // `test_builtin.py::BuiltinTest.test_import` — must raise
                // `TypeError`, not silently coerce the int via `.str()` and
                // go looking for a module literally named `"1"`).
                if !matches!(&*name_obj.borrow(), PyObject::Str(_)) {
                    return Err(PyError::type_error(
                        "__import__() argument 'name' must be str",
                    ));
                }
                let name = name_obj.str();
                // `__import__('')` (empty module name) is a real
                // `ValueError` in CPython, not "module not found" — but
                // ONLY for an absolute import (`level=0`, the default).
                // With `level>0` an empty name is the NORMAL, valid
                // encoding of a pure relative import (`from . import foo`
                // desugars to `__import__('', globals(), locals(),
                // ['foo'], 1)`) — `test_builtin.py::BuiltinTest.test_import`
                // exercises both: `__import__('')` (level 0, expects
                // ValueError) and a `level=1` call with fromlist (expects
                // ImportError from the relative-import-with-no-package
                // check, not ValueError).
                let level_kw = keywords
                    .iter()
                    .find(|(k, _)| k == "level")
                    .map(|(_, v)| v.clone());
                let level = args
                    .get(4)
                    .cloned()
                    .or(level_kw)
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);
                if name.is_empty() && level == 0 {
                    return Err(PyError::value_error("Empty module name"));
                }
                // `fromlist` is overwhelmingly passed as a KEYWORD argument
                // in real code (`__import__(name, fromlist=[...])` — real
                // trigger: CPython's own `dbm/__init__.py`), which under
                // this project's calling convention arrives as a trailing
                // packed kwargs dict, not a 4th positional argument — see
                // `object::builtin_import`'s matching doc comment for the
                // full story (checking only `args[3]` silently always
                // returned the top-level package instead of the requested
                // submodule, an infinite-recursion trap for callers that
                // then call `.open`/etc. on what they assumed was the
                // specific submodule).
                let kwargs_fromlist = keywords
                    .iter()
                    .find(|(k, _)| k == "fromlist")
                    .map(|(_, v)| v.clone());
                let fromlist_arg = kwargs_fromlist.or_else(|| args.get(3).cloned());
                let fromlist = fromlist_arg.and_then(|fl| match &*fl.borrow() {
                    PyObject::List(items) => Some(items.clone()),
                    PyObject::Tuple(items) => Some(items.iter().cloned().collect::<Vec<_>>()),
                    _ => None,
                });
                let has_dots = name.contains('.');
                let has_fromlist = fromlist
                    .as_ref()
                    .map_or(false, |fl: &Vec<PyObjectRef>| !fl.is_empty());
                return crate::object::import_impl(self, &name, has_dots, has_fromlist).map(Some);
            }
        }

        // `asyncio.run(coro)` — same `with_vm_mut`-is-unconditional-UB class
        // of bug (confirmed segfaulting via the simplest possible repro:
        // `asyncio.run(some_async_def())`, an extremely common real-world
        // async entry point). Shares `modules::asyncio_run_impl` (extracted
        // out of the inline closure for exactly this) with the real VM
        // directly.
        {
            let is_asyncio_run = matches!(&*callable.borrow(), PyObject::BuiltinFunction { func, .. } if std::ptr::fn_addr_eq(*func, crate::modules::asyncio_run_builtin as crate::object::BuiltinFunc));
            if is_asyncio_run && !args.is_empty() {
                return crate::modules::asyncio_run_impl(self, args[0].clone()).map(Some);
            }
        }

        // `signal.raise_signal(signum)` / `os.kill(pid, signum)` (own pid
        // only — the only pid meaningful in this single-process
        // interpreter) — actually CALLING a registered `signal.signal()`
        // handler needs a live `&mut VirtualMachine` (same class of bug as
        // `asyncio.run`/`start_new_thread` above). Confirmed via
        // `test_threadsignals.py`'s `acquire_retries_on_intr`, which relies
        // on `os.kill(os.getpid(), signal.SIGUSR1)` actually invoking the
        // handler registered via `signal.signal(signal.SIGUSR1, my_handler)`.
        {
            let is_raise_signal = matches!(&*callable.borrow(), PyObject::BuiltinFunction { func, .. } if std::ptr::fn_addr_eq(*func, crate::modules::signal_raise_signal_builtin as crate::object::BuiltinFunc));
            if is_raise_signal && !args.is_empty() {
                let signum = args[0]
                    .as_i64()
                    .ok_or_else(|| PyError::type_error("signalnum must be an int"))?;
                return crate::modules::signal_raise_signal_impl(self, signum).map(Some);
            }
            let is_os_kill = matches!(&*callable.borrow(), PyObject::BuiltinFunction { func, .. } if std::ptr::fn_addr_eq(*func, crate::modules::os_kill_builtin as crate::object::BuiltinFunc));
            if is_os_kill && args.len() >= 2 {
                let pid = args[0].as_i64().unwrap_or(-1);
                let signum = args[1]
                    .as_i64()
                    .ok_or_else(|| PyError::type_error("signalnum must be an int"))?;
                if pid == std::process::id() as i64 {
                    crate::modules::invoke_signal_handler_impl(self, signum)?;
                }
                return Ok(Some(py_none()));
            }
        }

        // `exec(source[, globals[, locals]])` / `eval(source[, globals[, locals]])`
        // — `object::builtin_exec`/`builtin_eval` (plain `fn(&[PyObjectRef])`,
        // no VM access) reached the VM via `with_vm_mut`, which grabs the
        // SAME `*mut VirtualMachine` this call is already executing under —
        // real aliasing UB (a second live `&mut self` to an object already
        // mutably borrowed by the current Rust call stack), not just "risky
        // in theory". `VM_PTR` is set unconditionally in `execute()` before
        // ANY bytecode runs, so this UB was hit by every `exec()`/`eval()`
        // call from normal running Python code, not just some rare nested
        // case — confirmed via the simplest possible repro (`exec("x = 1")`
        // at plain top level) reliably segfaulting. Fixed the same way as
        // `getattr`/`hasattr`/etc. above: run it through the real, live
        // `self` directly. Also fixes real semantics `with_vm_mut`'s
        // `vm.run(code)` never had: an explicit `globals`/`locals` dict
        // argument (needed by real code that generates functions via
        // `exec(src, globals_dict, locals_dict)` — CPython's own
        // `dataclasses.py` does exactly this) is now actually honored
        // instead of always executing against the top-level module globals.
        {
            let is_exec = matches!(&*callable.borrow(), PyObject::BuiltinFunction { func, .. } if std::ptr::fn_addr_eq(*func, crate::object::builtin_exec as crate::object::BuiltinFunc));
            let is_eval = matches!(&*callable.borrow(), PyObject::BuiltinFunction { func, .. } if std::ptr::fn_addr_eq(*func, crate::object::builtin_eval as crate::object::BuiltinFunc));
            if (is_exec || is_eval) && !args.is_empty() {
                let mode_name = if is_exec { "exec" } else { "eval" };
                let code = match &*args[0].borrow() {
                    PyObject::Code(c) => (**c).clone(),
                    _ => {
                        // `exec()`/`eval()` accept a `bytes`/`bytearray`
                        // source too (real CPython: PEP 263 coding-cookie
                        // decoded, defaulting to UTF-8) -- `.str()` on a
                        // Bytes object gives its Python REPR (`"b'...'"`),
                        // not the decoded text, which silently produced
                        // garbage source instead of decoding it properly
                        // (test_eof.py's test_line_continuation_EOF passes
                        // `'ä = 5\\'.encode()` and a latin-1-encoded
                        // `# coding:latin1` source directly).
                        let source = {
                            let b = args[0].borrow();
                            match &*b {
                                PyObject::Bytes(bytes) => {
                                    crate::object::import_builtin::decode_source_bytes(bytes)?
                                }
                                PyObject::ByteArray(bytes) => {
                                    crate::object::import_builtin::decode_source_bytes(bytes)?
                                }
                                _ => {
                                    drop(b);
                                    args[0].str()
                                }
                            }
                        };
                        // `eval()` compiles as a single EXPRESSION (returns
                        // its value via RETURN_VALUE) — `exec()` compiles as
                        // a statement list (returns None, matching module-
                        // level execution); using statement-mode for both
                        // (the pre-fix code's bug) made `eval("2+2")` return
                        // None instead of 4.
                        // A real `SyntaxError` (not `TypeError`) — see
                        // `PyError::syntax_error`'s own doc comment; same
                        // fix as `builtin_compile`'s equivalent parse sites.
                        // Use the real source text (`.text`) and correct
                        // filename here, not the bare `syntax_error(msg)`
                        // shorthand (which passes an EMPTY source string) —
                        // `exec()`/`eval()`'s raised SyntaxError otherwise
                        // always had `.text == ''` regardless of what the
                        // actual offending line was (test_eof.py's
                        // test_line_continuation_EOF: `exec('ä = 5\\')`
                        // needs `.text == 'ä = 5\\\n'`). `builtin_compile`'s
                        // own parse sites already do this correctly.
                        // The SyntaxError's own `.filename` is "<string>"
                        // regardless of exec-vs-eval (matches real CPython
                        // exactly); the CODE OBJECT's filename keeps the
                        // pre-existing "<exec>"/"<eval>" convention below,
                        // an unrelated, separate concern not touched here.
                        let program = if is_eval {
                            crate::parser::try_parse_as_expression(&source).map_err(|e| {
                                PyError::syntax_error_with_filename(e, "<string>", &source)
                            })?
                        } else {
                            let mut parser = crate::parser::Parser::new(&source, false);
                            parser.parse_program().map_err(|e| {
                                PyError::syntax_error_with_filename(e, "<string>", &source)
                            })?
                        };
                        let mut compiler = crate::compiler::Compiler::new();
                        compiler
                            .compile(&program, &format!("<{}>", mode_name))
                            .map_err(|e| {
                                PyError::syntax_error_with_filename(e, "<string>", &source)
                            })?
                    }
                };
                // Merge an explicit globals dict (reads) with an explicit
                // locals dict (reads take precedence, writes land here) into
                // one flat namespace — this interpreter's frames don't model
                // separate globals/locals scopes for top-level-style exec.
                // `isinstance(globals, dict)` in real CPython — a real
                // `dict` SUBCLASS (e.g. test_descrtut.py's `defaultdict`)
                // is a legal `exec()`/`eval()` globals/locals argument too,
                // not just an exact `dict`; it shows up here as a
                // `PyObject::Instance` carrying a native dict backing
                // (see `native_backing_of`), not a bare `PyObject::Dict`.
                let is_dict_like =
                    |v: &PyObjectRef| -> bool { crate::object::dict_arg_to_hashmap(v, "").is_ok() };
                // `globals`/`locals` are keyword-capable in real CPython —
                // `exec(src, globals=g)`, `eval(src, locals=data)` and
                // `exec(src, globals=g, locals=l)` all occur in real code
                // (test_builtin's `test_exec_kwargs`/`test_eval_kwargs`
                // exercise exactly these). `keywords` arrives separately
                // here, so merge the two sources positionally-first.
                let kw_arg = |name: &str| -> Option<PyObjectRef> {
                    keywords
                        .iter()
                        .find(|(k, _)| k == name)
                        .map(|(_, v)| v.clone())
                        .filter(|v| is_dict_like(v))
                };
                let globals_dict = args
                    .get(1)
                    .filter(|g| is_dict_like(g))
                    .cloned()
                    .or_else(|| kw_arg("globals"));
                let explicit_locals = args
                    .get(2)
                    .filter(|l| is_dict_like(l))
                    .cloned()
                    .or_else(|| kw_arg("locals"));
                let locals_dict = explicit_locals.clone().or_else(|| globals_dict.clone());
                let distinct_locals =
                    explicit_locals.filter(|l| globals_dict.as_ref().map_or(true, |g| !l.is(g)));
                // Real CPython injects `__builtins__` into an explicit
                // globals dict when missing — both so names resolve AND so
                // the key shows up iterating the dict afterwards (the
                // descrtut case: `exec("x = 3", a)` must leave `a` holding
                // `__builtins__`). Mirror the surrounding frame's own entry
                // so the object is identical to what a bare `exec(src)`
                // sees (name resolution would fall back to `f.builtins`
                // anyway, so this only adds the visible-dict behavior).
                let caller_builtins = self.frames.last().and_then(|f| {
                    f.globals
                        .borrow()
                        .get(&interner::intern("__builtins__"))
                        .cloned()
                });
                // CPython injects `__builtins__` into the EXPLICIT globals
                // mapping itself when it doesn't have one (that's why
                // `exec("z = 1", g)` leaves `g['__builtins__']` behind, and
                // every test_builtin exec case deletes it before comparing
                // the dict). Do it on the live mapping, not just the frame
                // copy, so `globals()['__builtins__']` and name resolution
                // both see it.
                if let (Some(g), Some(bm)) = (globals_dict.as_ref(), caller_builtins.clone()) {
                    let has_builtins =
                        crate::object::py_getitem(g, &py_str("__builtins__")).is_ok();
                    if !has_builtins {
                        let _ = crate::object::py_setitem(g, &py_str("__builtins__"), bm);
                    }
                }
                let namespace = if let Some(g) = globals_dict.as_ref() {
                    if let Some(l) = distinct_locals.as_ref() {
                        let mut hm: HashMap<StrId, PyObjectRef> = str_map_to_strid_map(
                            crate::object::dict_arg_to_hashmap(g, "exec() globals must be a dict")?,
                        );
                        hm.extend(str_map_to_strid_map(crate::object::dict_arg_to_hashmap(
                            &l,
                            "exec() locals must be a dict",
                        )?));
                        if let Some(bm) = caller_builtins.clone() {
                            hm.entry(interner::intern("__builtins__")).or_insert(bm);
                        }
                        Some(Rc::new(RefCell::new(hm)))
                    } else {
                        // No separate locals dict: `g` is both the read AND
                        // write-back target, exactly the shape `exec(code,
                        // shared_dict)` called repeatedly against the SAME
                        // dict relies on (doctest, `code.InteractiveConsole`,
                        // any REPL-like tool) — reuse a persistent namespace
                        // keyed by `g`'s identity instead of a fresh
                        // snapshot each time (see `exec_globals_cache`'s doc
                        // comment for why a fresh snapshot breaks closures).
                        let key = g.get_id();
                        let ns = match self.exec_globals_cache.get(&key) {
                            Some((cached, ns)) if cached.is(g) => ns.clone(),
                            _ => {
                                let ns = Rc::new(RefCell::new(HashMap::new()));
                                self.exec_globals_cache.insert(key, (g.clone(), ns.clone()));
                                ns
                            }
                        };
                        {
                            let mut hm = ns.borrow_mut();
                            hm.clear();
                            for (k, v) in crate::object::dict_arg_to_hashmap(
                                g,
                                "exec() globals must be a dict",
                            )? {
                                hm.insert(interner::intern(&k), v);
                            }
                            if let Some(bm) = caller_builtins.clone() {
                                hm.entry(interner::intern("__builtins__")).or_insert(bm);
                            }
                        }
                        Some(ns)
                    }
                } else if is_eval {
                    // `eval(source)` with no globals/locals args: CPython
                    // defaults both to the CALLING frame's globals AND
                    // locals, so an expression can read the enclosing
                    // function's parameters/locals (`eval('args[1] is not
                    // None')` inside `def check(*args)` — real trigger:
                    // CPython's own test_decorators.py's `dbcheck` helper).
                    // Build a read-only snapshot: the frame's globals merged
                    // with its fast-locals by varname.
                    if let Some(f) = self.frames.last() {
                        let mut hm: HashMap<StrId, PyObjectRef> = f
                            .globals
                            .borrow()
                            .iter()
                            .map(|(k, v)| (*k, v.clone()))
                            .collect();
                        for (i, name) in f.code.varnames.iter().enumerate() {
                            if let Some(v) = f.fast_locals.get(i).and_then(|v| v.clone()) {
                                hm.insert(*name, v);
                            }
                        }
                        Some(Rc::new(RefCell::new(hm)))
                    } else {
                        None
                    }
                } else if is_exec && args.len() == 1 {
                    // Bare `exec(source)` in function scope: merge the
                    // caller frame's globals, fast-locals (by varname)
                    // and frame locals into one namespace so reads
                    // resolve (mirrors the bare-`eval` snapshot above
                    // plus `locals()`' frame-locals merge). Writes stay
                    // contained in the copy, matching CPython's
                    // exec-in-function gotcha. Module scope (no
                    // populated fast locals) keeps the live globals
                    // below so writes stay visible. Explicit-non-dict
                    // globals args keep the historical fallthrough.
                    if let Some(f) = self.frames.last() {
                        if f.fast_locals.iter().any(|s| s.is_some()) {
                            let mut hm: HashMap<StrId, PyObjectRef> = f
                                .globals
                                .borrow()
                                .iter()
                                .map(|(k, v)| (*k, v.clone()))
                                .collect();
                            for (i, name) in f.code.varnames.iter().enumerate() {
                                if let Some(v) = f.fast_locals.get(i).and_then(|v| v.clone()) {
                                    hm.insert(*name, v);
                                }
                            }
                            for (k, v) in f.locals.iter() {
                                hm.insert(interner::intern(&crate::interner::lookup(k)), v.clone());
                            }
                            Some(Rc::new(RefCell::new(hm)))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                };
                let globals_rc = namespace.clone().unwrap_or_else(|| {
                    self.frames
                        .last()
                        .map(|f| f.globals.clone())
                        .unwrap_or_else(|| self.globals.clone())
                });
                // A DISTINCT locals mapping runs live on the frame
                // (Frame::exec_locals): reads check it before globals,
                // stores/deletes land in it directly, so no mirror-back
                // is needed — and mirroring would wrongly copy the
                // globals-seeded entries into it. The shared-namespace
                // shape below (no distinct locals) still needs the
                // mirror to write results back into the caller's dict.
                let live_locals = if distinct_locals.is_some() {
                    locals_dict.clone()
                } else {
                    None
                };
                let result = match live_locals.clone() {
                    Some(l) => self.exec_code_with_mappings(
                        code,
                        Some(globals_rc.clone()),
                        Some(l),
                        globals_dict.clone(),
                    ),
                    None => self.exec_code_with_mappings(
                        code,
                        Some(globals_rc.clone()),
                        None,
                        globals_dict.clone(),
                    ),
                };
                if live_locals.is_none() {
                    if let Some(target) = locals_dict {
                        let real_target = if matches!(&*target.borrow(), PyObject::Dict(_)) {
                            target.clone()
                        } else {
                            crate::object::native_backing_of(&target)
                                .unwrap_or_else(|| target.clone())
                        };
                        let mut real_target_borrow = real_target.borrow_mut();
                        if let PyObject::Dict(d) = &mut *real_target_borrow {
                            // Full mirror, not a merge: `globals_rc` started as a
                            // complete copy of `target`'s own contents (see
                            // `namespace`'s construction above), so any name
                            // present in `target` but no longer in `globals_rc`
                            // was `del`eted during execution and must disappear
                            // from `target` too (test_descrtut.py's `del
                            // property  # unmask the builtin` doctest).
                            // Non-string-keyed entries (e.g. int keys on a
                            // dict-subclass exec namespace) cannot round-trip
                            // through the StrId-keyed frame namespace — and
                            // Python code under exec cannot name them either —
                            // so save them aside and restore verbatim. The old
                            // code wrote every key back stringified, permanently
                            // corrupting the caller's dict (int key 1 became
                            // '1', test_descrtut).
                            let saved_non_string: Vec<(PyObjectRef, PyObjectRef)> = d
                                .items()
                                .into_iter()
                                .filter(|(k, _)| !matches!(&*k.borrow(), PyObject::Str(_)))
                                .collect();
                            // String forms of the saved keys above: these strings
                            // may ALSO be present in the frame namespace purely
                            // because the read path stringified the same
                            // non-string keys (`dict_arg_to_hashmap` maps
                            // `k.str()`). They must not be mirrored back, or the
                            // caller ends up with BOTH `1` and `'1'`. (An exec
                            // body explicitly assigning `globals()["1"]` is the
                            // only collision, and vanishingly rare versus the
                            // always-on corruption.)
                            let shadowed: std::collections::HashSet<String> =
                                saved_non_string.iter().map(|(k, _)| k.str()).collect();
                            d.clear();
                            for (k, v) in globals_rc.borrow().iter() {
                                let s = interner::lookup_str(*k);
                                if shadowed.contains(s) {
                                    continue;
                                }
                                let _ = d.set(py_str(s), v.clone());
                            }
                            for (k, v) in saved_non_string {
                                let _ = d.set(k, v);
                            }
                        }
                    }
                }
                return match result {
                    Ok(val) => {
                        // Coroutine/generator-producing eval/exec: the
                        // returned object resumes later via send/next, so
                        // stamp its stored frame with the live user-dict
                        // link now — each resume then mirrors writes back
                        // (CPython: the namespace IS that dict). Without
                        // this, `eval(coro_code, ns)` + driving leaves
                        // `ns` stale (test_builtin's await-eval half of
                        // test_compile_top_level_await).
                        if let PyObject::Coroutine { frame } | PyObject::Generator { frame } =
                            &*val.borrow()
                        {
                            if let Ok(mut guard) = frame.try_borrow_mut() {
                                if let Some(f) = guard.as_mut() {
                                    if f.globals_dict.is_none() {
                                        f.globals_dict = globals_dict.clone();
                                    }
                                    if f.exec_globals.is_none() {
                                        f.exec_globals = globals_dict.clone();
                                    }
                                }
                            }
                        }
                        Ok(Some(if is_exec { py_none() } else { val }))
                    }
                    Err(e) => Err(e),
                };
            }
        }

        // `importlib.import_module(name, package=None)` — same reasoning
        // as `getattr` just above: its own implementation normally reaches
        // the VM only via `with_vm_mut`, a second aliasing `&mut self`
        // while this exact call chain already holds one. Real code calls
        // this constantly (Django's own `django.utils.module_loading.
        // import_string` — used to resolve `LOGGING_CONFIG =
        // "logging.config.dictConfig"` and similar dotted-path settings —
        // goes through `importlib.import_module` for the module half of
        // the path), so route it through the live `self` directly instead.
        {
            let is_import_module = matches!(&*callable.borrow(), PyObject::BuiltinFunction { func, .. } if std::ptr::fn_addr_eq(*func, crate::modules::import_module_builtin as crate::object::BuiltinFunc));
            if is_import_module && !args.is_empty() {
                let name = args[0].str();
                let package = if args.len() >= 2 {
                    let pkg = args[1].str();
                    if pkg.is_empty() {
                        None
                    } else {
                        Some(pkg)
                    }
                } else {
                    None
                };
                return crate::modules::import_module_with_vm(self, &name, package.as_deref())
                    .map(Some);
            }
        }

        // `importlib.util.find_spec` (`find_spec_builtin`) internally used
        // `with_vm_mut` to read `vm.modules`/`sys.path` — reached constantly
        // from deep inside an active VM call chain in practice (e.g. Django's
        // app registry calls it while `apps.populate()` is running), which
        // reborrows the *same* live VirtualMachine `with_vm_mut` already has
        // a `&mut self` for elsewhere on the Rust call stack: real aliasing
        // UB, confirmed via a non-deterministic segfault/corrupted-HashMap
        // crash (not just theoretical). Route it through the real, live
        // `&mut self` directly instead, same pattern as getattr/import_module
        // above.
        {
            let is_find_spec = matches!(&*callable.borrow(), PyObject::BuiltinFunction { func, .. } if std::ptr::fn_addr_eq(*func, crate::modules::find_spec_builtin as crate::object::BuiltinFunc));
            if is_find_spec && !args.is_empty() {
                let name = args[0].str();
                let package = if args.len() >= 2 {
                    let pkg = args[1].str();
                    if pkg.is_empty() {
                        None
                    } else {
                        Some(pkg)
                    }
                } else {
                    None
                };
                return crate::modules::find_spec_with_vm(self, &name, package.as_deref())
                    .map(Some);
            }
        }

        // `inspect.getmembers(obj, predicate)` needs to actually CALL
        // `predicate` on each candidate member — same reentrancy hazard as
        // find_spec above (reached from deep inside Django's app-loading:
        // `inspect.getmembers(mod, inspect.isclass)`), so route it through
        // the real, live `&mut self` directly instead of a disposable VM.
        {
            let is_getmembers = matches!(&*callable.borrow(), PyObject::BuiltinFunction { func, .. } if std::ptr::fn_addr_eq(*func, crate::modules::getmembers_builtin as crate::object::BuiltinFunc));
            if is_getmembers && !args.is_empty() {
                let predicate = args.get(1).cloned();
                return crate::modules::getmembers_with_vm(self, &args[0], predicate.as_ref())
                    .map(Some);
            }
        }

        // `pickle.loads`/`_pickle.loads`/`pickle.load`/`Unpickler.load`
        // resolving a pickled `GLOBAL` opcode reference to a module that
        // hasn't been imported yet (real CPython's `find_class` does
        // `__import__(module)` then `getattr(sys.modules[module], name)` —
        // `test_pickle.py::test_unpickle_module_race`'s
        // `locking_import.ToBeUnpickled` is exactly this case) needs a real
        // import, which needs the live `&mut VirtualMachine`, same
        // reentrancy hazard as the others above.
        {
            let is_pickle_loads = matches!(&*callable.borrow(), PyObject::BuiltinFunction { func, .. } if std::ptr::fn_addr_eq(*func, crate::modules::pickle_loads_plain as crate::object::BuiltinFunc));
            if is_pickle_loads && !args.is_empty() {
                return crate::modules::pickle_loads_with_vm(self, args).map(Some);
            }
            let is_pickle_load = matches!(&*callable.borrow(), PyObject::BuiltinFunction { func, .. } if std::ptr::fn_addr_eq(*func, crate::modules::pickle_load_plain as crate::object::BuiltinFunc));
            if is_pickle_load && !args.is_empty() {
                return crate::modules::pickle_load_with_vm(self, args).map(Some);
            }
            let is_unpickler_load = matches!(&*callable.borrow(), PyObject::BuiltinFunction { func, .. } if std::ptr::fn_addr_eq(*func, crate::modules::unpickler_load_plain as crate::object::BuiltinFunc));
            if is_unpickler_load && !args.is_empty() {
                return crate::modules::unpickler_load_with_vm(self, args).map(Some);
            }
        }

        Ok(None)
    }
}
