use crate::bytecode::Opcode;
use crate::object::*;
use crate::vm::helpers::deref_proxy;
use crate::vm::VirtualMachine;
use std::rc::Rc;

impl VirtualMachine {
    pub(crate) fn handle_attr(&mut self, fi: usize, op: Opcode, arg: u32) -> PyResult<bool> {
        match op {
            Opcode::LOAD_ATTR => {
                let name_idx = arg as usize;
                let name = crate::interner::lookup(self.frames[fi].code.names[name_idx]);
                let obj = self.frames[fi].pop()?;
                let obj = deref_proxy(&obj)?;
                let result = {
                    let obj_borrowed = obj.borrow();
                    match &*obj_borrowed {
                        // `obj` `PyObjectRef` is available to bind directly
                        // — same fix shape as `GET_AWAITABLE`'s own
                        // `self_obj`-rebind elsewhere in this file, for the
                        // identical underlying limitation.
                        PyObject::ListIter { .. }
                        | PyObject::RangeIter { .. }
                        | PyObject::MapIterator { .. }
                        | PyObject::FilterIterator { .. }
                        | PyObject::ZipIterator { .. }
                        | PyObject::CycleIter { .. }
                        | PyObject::GroupByIter { .. }
                        | PyObject::EnumerateIter { .. }
                        | PyObject::GetItemIter { .. }
                        | PyObject::CallSentinelIter { .. }
                            if name == "__next__" || name == "__iter__" =>
                        {
                            let func: crate::object::BuiltinFunc = if name == "__next__" {
                                crate::object::builtin_next
                            } else {
                                crate::object::builtin_iter
                            };
                            Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                                name: name.clone(),
                                func,
                                self_obj: obj.clone(),
                            }))
                        }
                        // `range_iterator`/`list_iterator.__setstate__(state)`
                        // (real CPython's pickle-restore protocol, also
                        // directly usable) — needs the same real-`self_obj`
                        // treatment as `__next__`/`__iter__` just above,
                        // since it MUTATES the iterator's position in place
                        // (a disconnected clone would silently do nothing).
                        // Found via CPython's own `test_range.py::
                        // test_iterator_setstate`.
                        PyObject::RangeIter { .. } if name == "__setstate__" => {
                            Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                                name: name.clone(),
                                func: crate::object::range_iter_setstate,
                                self_obj: obj.clone(),
                            }))
                        }
                        PyObject::ListIter { .. } if name == "__setstate__" => {
                            Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                                name: name.clone(),
                                func: crate::object::list_iter_setstate,
                                self_obj: obj.clone(),
                            }))
                        }
                        PyObject::Super {
                            cls: _,
                            obj: _super_obj,
                        } => {
                            // super(cls, obj).attr: walk MRO of obj's type, starting after cls
                            drop(obj_borrowed);
                            let attr = obj.borrow().get_attribute(&name)?;
                            Ok(attr)
                        }
                        PyObject::Function(_) if name == "__dict__" => {
                            // `func.__dict__` as a LIVE view backed by the
                            // function's real dict (writes through via
                            // `PyDict::set_with_hash`'s Function arm) — the
                            // get_attribute_impl snapshot copy made the common
                            // `func.__dict__['x'] = v` decorator-helper
                            // pattern silently write into a discarded copy.
                            // Mirrors the Instance `__dict__` live-view right
                            // below.
                            let mut pd = crate::object::PyDict::new();
                            if let PyObject::Function(f) = &*obj.borrow() {
                                for (k, v) in f.dict.iter() {
                                    if k.starts_with("__") && k.ends_with("__") {
                                        continue;
                                    }
                                    let _ = pd.set(py_str(k), v.clone());
                                }
                            }
                            drop(obj_borrowed);
                            pd.instance_ref = Some(obj.clone());
                            self.frames[fi].push(PyObjectRef::new(PyObject::Dict(Box::new(pd))));
                            return Ok(true);
                        }
                        PyObject::Module { .. } if name == "__dict__" => {
                            let mut pd = crate::object::PyDict::new();
                            if let PyObject::Module { dict, .. } = &*obj.borrow() {
                                for (k, v) in dict.iter() {
                                    let key = py_str(crate::interner::lookup_str(*k));
                                    let _ = pd.set(key, v.clone());
                                }
                            }
                            drop(obj_borrowed);
                            pd.instance_ref = Some(obj.clone());
                            self.frames[fi].push(PyObjectRef::new(PyObject::Dict(Box::new(pd))));
                            return Ok(true);
                        }
                        PyObject::Instance { dict, typ } => {
                            // Inline attribute cache: skip full lookup if cached
                            // with matching type tag — only valid when this
                            // instance's OWN dict doesn't also define `name`,
                            // since the cache only ever stores type/mro-level
                            // hits (methods, class attributes); an instance
                            // that shadows the class attribute with its own
                            // instance-level value of the same name must still
                            // win over a stale cache entry from some OTHER
                            // instance of the same type that didn't have that
                            // override (see the caching-site comment below for
                            // the matching write-side half of this fix).
                            let type_tag = typ.get_id() as u64;
                            let cached = if dict.contains_key(&name) {
                                None
                            } else {
                                self.frames[fi]
                                    .attr_cache
                                    .get(name_idx)
                                    .and_then(|entry| entry.as_ref())
                                    .filter(|(tag, _)| *tag == type_tag)
                                    .map(|(_, val)| val.clone())
                            };
                            if let Some(cached_val) = cached {
                                // The cached value may be a method already
                                // BOUND to whatever instance first populated
                                // this cache slot (`self_obj` baked in) — the
                                // cache itself is keyed only by
                                // `(name_idx, type_tag)`, with no per-
                                // instance component, so reusing it AS-IS
                                // for a DIFFERENT instance of the same type
                                // silently operated on the wrong `self`
                                // (confirmed via a direct repro: calling the
                                // same bound-method-shaped attribute on two
                                // different instances of the same class
                                // within one frame — e.g.
                                // `subprocess.CompletedProcess.check_returncode`
                                // — the second call silently used the
                                // FIRST instance as `self`). Rebind to the
                                // CURRENT `obj` before returning, matching
                                // the same rebind-on-hit fix already applied
                                // to the OTHER (module-level) attribute
                                // cache just above in this file.
                                //
                                // Only rebind when the cached `self_obj` is
                                // ITSELF an `Instance` of this SAME type —
                                // i.e. unambiguously "some OTHER instance of
                                // the identical class", the exact cross-
                                // instance-pollution case above. A cached
                                // method deliberately bound to something
                                // ELSE (native-backing delegation for a
                                // class transparently subclassing list/
                                // dict/str, or any other legitimate "bound
                                // to a fixed, different object" case) must
                                // be returned UNCHANGED — rebinding it
                                // unconditionally to `obj` broke exactly
                                // that (confirmed regression: `collections`'
                                // own `Counter.update` internals, which rely
                                // on a cached method staying bound to its
                                // real native-backing dict rather than the
                                // wrapper `Instance`).
                                let rebound = match &*cached_val.borrow() {
                                    PyObject::BuiltinMethod {
                                        name: n,
                                        func,
                                        self_obj,
                                    } if matches!(&*self_obj.borrow(), PyObject::Instance { typ: t, .. } if t.is(typ)) => {
                                        PyObjectRef::imm(PyObject::BuiltinMethod {
                                            name: n.clone(),
                                            func: *func,
                                            self_obj: obj.clone(),
                                        })
                                    }
                                    PyObject::BoundMethod { func, self_obj } if matches!(&*self_obj.borrow(), PyObject::Instance { typ: t, .. } if t.is(typ)) => {
                                        PyObjectRef::imm(PyObject::BoundMethod {
                                            func: func.clone(),
                                            self_obj: obj.clone(),
                                        })
                                    }
                                    _ => cached_val.clone(),
                                };
                                self.frames[fi].push(rebound);
                                return Ok(true);
                            }
                            if name == "__dict__" {
                                // Return a live Dict view backed by the instance's HashMap.
                                // NATIVE_BACKING_KEY is internal bookkeeping
                                // (see native_backing_of) and must not leak
                                // into user-visible introspection.
                                let mut pd = crate::object::PyDict::new();
                                for (k, v) in dict.iter() {
                                    if k == crate::object::NATIVE_BACKING_KEY {
                                        continue;
                                    }
                                    let key = py_str(k);
                                    pd.set(key, v.clone())?;
                                }
                                drop(obj_borrowed);
                                pd.instance_ref = Some(obj.clone());
                                self.frames[fi]
                                    .push(PyObjectRef::new(PyObject::Dict(Box::new(pd))));
                                return Ok(true);
                            }
                            if name == "__class__" {
                                let cls = typ.clone();
                                drop(obj_borrowed);
                                self.frames[fi].push(cls);
                                return Ok(true);
                            }
                            // Clone dict/typ into owned values and drop the
                            // borrow of `obj` ITSELF now — the descriptor
                            // dispatch below may call into arbitrary Python
                            // code (a `@property` getter, `cached_property`'s
                            // `__get__`, etc.), and such code very commonly
                            // writes back into `instance.__dict__` (that's
                            // literally what `cached_property.__get__` does,
                            // to cache its computed value for next time) —
                            // if `obj`'s own borrow were still held here, that
                            // nested write's borrow_mut() on the SAME RefCell
                            // panics the moment such a getter touches the
                            // instance it was called on (confirmed via a
                            // genuine, general, Django-free repro).
                            let dict: crate::object::AttrMap = dict.clone();
                            let typ: PyObjectRef = typ.clone();
                            drop(obj_borrowed);
                            let dict = &dict;
                            let typ = &typ;
                            let attr = if let Some(inst_attr) = dict.get_str(&name) {
                                Ok(Some(inst_attr.clone()))
                            } else {
                                (|| -> PyResult<Option<PyObjectRef>> {
                                    let typ_ref = typ.borrow();
                                    if let PyObject::Type {
                                        dict: type_dict,
                                        mro,
                                        ..
                                    } = &*typ_ref
                                    {
                                        let found =
                                            type_dict.get_str(&name).cloned().or_else(|| {
                                                for base in mro.iter().skip(1) {
                                                    if let PyObject::Type {
                                                        dict: base_dict, ..
                                                    } = &*base.borrow()
                                                    {
                                                        if let Some(val) = base_dict.get_str(&name)
                                                        {
                                                            return Some(val.clone());
                                                        }
                                                    }
                                                }
                                                None
                                            });
                                        // Handle descriptor protocol for Property, StaticMethod, ClassMethod, and generic __get__
                                        if let Some(val) = found {
                                            let val_borrowed = val.borrow();
                                            match &*val_borrowed {
                                            PyObject::Property(ref d) if d.getter.is_some() => {
                                                drop(typ_ref);
                                                let g = d.getter.as_ref().unwrap();
                                                return Ok(Some(self.call_function(g.clone(), vec![obj.clone()], vec![]).unwrap_or_else(|_| val.clone())));
                                            }
                                            PyObject::StaticMethod { func } => {
                                                return Ok(Some(func.clone()));
                                            }
                                            PyObject::ClassMethod { func } => {
                                                let func_clone = func.clone();
                                                drop(val_borrowed);
                                                drop(typ_ref);
                                                let cls = obj.borrow();
                                                if let PyObject::Instance { typ: inst_typ, .. } = &*cls {
                                                    // Return a BoundMethod that will prepend the class when called
                                                    let class_obj = inst_typ.clone();
                                                    drop(cls);
                                                    return Ok(Some(PyObjectRef::imm(PyObject::BoundMethod {
                                                        func: func_clone,
                                                        self_obj: class_obj,
                                                    })));
                                                }
                                                // When accessing classmethod on a type itself (e.g. MyClass.method),
                                                // bind the type as self so it becomes the first arg on call
                                                let class_obj = obj.clone();
                                                drop(cls);
                                                return Ok(Some(PyObjectRef::imm(PyObject::BoundMethod {
                                                    func: func_clone,
                                                    self_obj: class_obj,
                                                })));
                                            }
                                            PyObject::Function(_) => {
                                                let is_instance_obj = matches!(&*obj.borrow(), PyObject::Instance { .. });
                                                if is_instance_obj {
                                                    return Ok(Some(PyObjectRef::imm(PyObject::BoundMethod {
                                                        func: val.clone(),
                                                        self_obj: obj.clone(),
                                                    })));
                                                } else {
                                                    return Ok(Some(val.clone()));
                                                }
                                            }
                                            PyObject::BuiltinFunction { name: n, func }
                                                if crate::object::is_builtin_exception_class_name(n)
                                                    || std::ptr::fn_addr_eq(*func, crate::object::builtin_open as crate::object::BuiltinFunc) =>
                                            {
                                                // Do NOT auto-bind a builtin
                                                // exception "class" (this
                                                // codebase's representation for
                                                // e.g. `AssertionError`) found
                                                // as a plain class attribute
                                                // (`failureException =
                                                // AssertionError`) — unlike a
                                                // genuine native METHOD (also a
                                                // `BuiltinFunction`, e.g.
                                                // `hmac`'s `HMAC.hexdigest`,
                                                // which DOES rely on `self`
                                                // being auto-prepended — see
                                                // the `else` arm just below,
                                                // unchanged for that case), a
                                                // class reference is never a
                                                // descriptor in real Python, so
                                                // binding it here silently
                                                // prepended `self` as an extra
                                                // positional argument to every
                                                // call and turned the class
                                                // reference into a `BoundMethod`
                                                // that `issubclass()` could no
                                                // longer recognize as a class at
                                                // all — confirmed via
                                                // `unittest`'s own
                                                // `self.failureException(msg)`
                                                // (raising `AssertionError(self,
                                                // msg)` instead of
                                                // `AssertionError(msg)`) and
                                                // `issubclass(exc_info[0],
                                                // test.failureException)`
                                                // (always False, misclassifying
                                                // every real test failure as an
                                                // error).
                                                let _ = func;
                                                return Ok(Some(val.clone()));
                                            }
                                            PyObject::BuiltinFunction { name: n, func } => {
                                                let n = n.clone();
                                                let func = *func;
                                                // Plain builtin module functions
                                                // (`isclose = math.isclose`) have no
                                                // `__get__` and must stay UNBOUND — a
                                                // genuinely bound native method lives
                                                // only in its type's dict, never in a
                                                // module namespace. Drop the borrow
                                                // first: if `val` IS a module member,
                                                // the scan below re-borrows the same
                                                // RefCell.
                                                drop(val_borrowed);
                                                if self.is_plain_module_function(&n, &val) {
                                                    return Ok(Some(val.clone()));
                                                }
                                                return Ok(Some(PyObjectRef::imm(PyObject::BuiltinMethod {
                                                    name: n,
                                                    func,
                                                    self_obj: obj.clone(),
                                                })));
                                            }
                                            PyObject::BuiltinMethod { name: n, func, .. } => {
                                                return Ok(Some(PyObjectRef::imm(PyObject::BuiltinMethod {
                                                    name: n.clone(),
                                                    func: *func,
                                                    self_obj: obj.clone(),
                                                })));
                                            }
                                            // NOTE: deliberately NOT auto-binding a bare
                                            // `PyObject::Closure` found via the class dict here
                                            // — unlike `Function`/`BuiltinFunction` just above,
                                            // `Closure` is ALSO used pervasively for the
                                            // opposite convention: a closure built FRESH per
                                            // instance (e.g. `io.BytesIO`'s `read`/`write`/
                                            // `seek`, `dev.rs`), capturing that instance's own
                                            // state directly and expecting NO `self` prepended
                                            // at all. Auto-binding unconditionally here broke
                                            // those (their first REAL argument became `self`
                                            // instead) — confirmed via `io.BytesIO().write(b"x")`
                                            // regressing to `TypeError: a bytes-like object is
                                            // required, not str`. A shared, TYPE-level `Closure`
                                            // needing `self` (e.g. `namedtuple`'s own
                                            // `_asdict`/`_replace`) should instead be
                                            // implemented as a plain `BuiltinFunction` reading
                                            // whatever state it needs off `self` at call time —
                                            // that convention already auto-binds correctly via
                                            // the arm above, with no ambiguity.
                                            _ => {
                                                // Generic descriptor protocol: if value has __get__, call it
                                                drop(val_borrowed);
                                                let cls = {
                                                    let owner_type = obj.borrow();
                                                    if let PyObject::Instance { typ: inst_typ, .. } = &*owner_type {
                                                        Some(inst_typ.clone())
                                                    } else {
                                                        None
                                                    }
                                                };
                                                if let Some(cls) = cls {
                                                    if let Ok(__get__) = val.borrow().get_attribute("__get__") {
                                                        if std::env::var("RPY_DEBUG_DESCRIPTOR2").is_ok() {
                                                            eprintln!("INSTANCE-LEVEL __get__: attr_name={} val_type={:?} obj_type={:?}", name, val.borrow().type_name(), obj.borrow().type_name());
                                                        }
                                                        let descriptor_args = vec![val.clone(), obj.clone(), cls];
                                                        match self.call_function(__get__, descriptor_args, vec![]) {
                                                            Ok(v) => return Ok(Some(v)),
                                                            Err(e) => return Err(e),
                                                        }
                                                    }
                                                }
                                                return Ok(Some(val.clone()));
                                            }
                                        }
                                        }
                                        Ok(None)
                                    } else {
                                        Ok(None)
                                    }
                                })()
                            };
                            let attr = attr?;
                            // Not overridden anywhere in the mro: for a class
                            // that transparently subclasses list/dict/str
                            // (`class Foo(list): ...`), delegate to the real
                            // native value's own attribute resolution, rebound
                            // to the native backing (not this instance) since
                            // that's the object whose state actually mutates.
                            // Must run BEFORE the generic dict-like fallback
                            // below, which would otherwise misinterpret the
                            // native backing's own dict entry as plain
                            // instance-attribute data.
                            let attr = attr.or_else(|| {
                                let native = dict.get(crate::object::NATIVE_BACKING_KEY)?;
                                // A deque subclass's `__copy__`/`copy()` must
                                // return a NEW instance of the SAME subclass
                                // (not a raw deque) — build that closure here,
                                // since this inline resolution path mirrors
                                // `get_attribute_impl`'s own handling.
                                if matches!(&*native.borrow(), PyObject::Deque { .. })
                                    && (name == "__copy__" || name == "copy")
                                {
                                    let typ_clone = typ.clone();
                                    let new_native = {
                                        let b = native.borrow();
                                        if let PyObject::Deque { data, maxlen } = &*b {
                                            py_deque(data.clone(), *maxlen)
                                        } else {
                                            unreachable!()
                                        }
                                    };
                                    return Some(PyObjectRef::new(PyObject::Closure(Rc::new(
                                        move |_args: &[PyObjectRef]| -> PyResult<PyObjectRef> {
                                            let mut new_dict = crate::object::AttrMap::new();
                                            new_dict.insert(
                                                crate::object::NATIVE_BACKING_KEY.to_string(),
                                                new_native.clone(),
                                            );
                                            Ok(PyObjectRef::new(PyObject::Instance {
                                                typ: typ_clone.clone(),
                                                dict: new_dict,
                                            }))
                                        },
                                    ))));
                                }
                                let val = native.borrow().get_attribute(&name).ok()?;
                                let rebound = match &*val.borrow() {
                                    PyObject::BuiltinMethod { name: n, func, .. } => {
                                        Some(PyObjectRef::imm(PyObject::BuiltinMethod {
                                            name: n.clone(),
                                            func: *func,
                                            self_obj: native.clone(),
                                        }))
                                    }
                                    _ => None,
                                };
                                Some(rebound.unwrap_or(val))
                            });
                            // Fallback for dict methods on dict-derived instances
                            let attr = attr.or_else(|| {
                                if name == "__iter__"
                                    || name == "items"
                                    || name == "keys"
                                    || name == "values"
                                    || name == "get"
                                {
                                    let func: crate::object::BuiltinFunc = match name.as_str() {
                                        "__iter__" => crate::object::dict_method_iter,
                                        "items" => crate::object::dict_method_items,
                                        "keys" => crate::object::dict_method_keys,
                                        "values" => crate::object::dict_method_values,
                                        "get" => crate::object::dict_method_get,
                                        _ => return None,
                                    };
                                    Some(PyObjectRef::imm(PyObject::BuiltinMethod {
                                        name: name.clone(),
                                        func,
                                        self_obj: obj.clone(),
                                    }))
                                } else {
                                    None
                                }
                            });
                            // PEP 3134 traceback/chaining protocol methods
                            // for a user-defined exception class that
                            // doesn't override them — same fix, same
                            // rationale, as the `get_attribute_impl` copy of
                            // this logic (`object.rs`); this is LOAD_ATTR's
                            // own separate, inline copy of instance
                            // attribute resolution (kept for its attribute
                            // cache), which needs the identical fallback.
                            let attr = attr.or_else(|| {
                                if matches!(name.as_str(), "with_traceback" | "add_note" | "__traceback__" | "__context__" | "__cause__" | "__suppress_context__" | "__notes__")
                                    && crate::object::find_exception_base_name(typ).is_some() {
                                    Some(match name.as_str() {
                                        "with_traceback" => PyObjectRef::imm(PyObject::BuiltinMethod {
                                            name: "with_traceback".to_string(),
                                            func: |args| {
                                                if args.is_empty() { return Err(PyError::type_error("with_traceback() takes exactly one argument")); }
                                                Ok(args[0].clone())
                                            },
                                            self_obj: obj.clone(),
                                        }),
                                        "add_note" => PyObjectRef::imm(PyObject::BuiltinMethod {
                                            name: "add_note".to_string(),
                                            func: |_args| Ok(py_none()),
                                            self_obj: obj.clone(),
                                        }),
                                        // See the matching fix (and its full
                                        // explanation) in `get_attribute_impl`'s
                                        // copy of this same list (`attrs.rs`) —
                                        // `__cause__` was missing from both.
                                        "__context__" | "__traceback__" | "__cause__" => py_none(),
                                        "__suppress_context__" => py_bool(false),
                                        "__notes__" => py_list(vec![]),
                                        _ => unreachable!(),
                                    })
                                } else {
                                    None
                                }
                            });
                            match attr {
                                Some(val) => {
                                    // Cache attribute for future accesses — but
                                    // ONLY when it was found on the TYPE's own
                                    // dict/mro (a method or class attribute,
                                    // identical for every instance of this
                                    // type), never when it came from the
                                    // INSTANCE's own dict. A plain instance
                                    // attribute (`self.v`) varies per-instance,
                                    // but this cache is keyed only by
                                    // `(name_idx, type_tag)` — with no
                                    // per-instance component at all — so
                                    // caching an instance-dict hit here meant
                                    // ANY second instance of the same type
                                    // accessed via the same attribute name
                                    // within the same frame (e.g. `self.v` vs
                                    // `other.v` inside `__lt__(self, other)`)
                                    // silently got back the FIRST instance's
                                    // value instead of its own — a severe,
                                    // general correctness bug, not merely a
                                    // missed-cache-hit inefficiency. Confirmed
                                    // via a minimal repro: `other.v` returning
                                    // `self.v`'s value inside a two-argument
                                    // comparison method.
                                    //
                                    // A `property`'s (or any other `__get__`-
                                    // based descriptor's) getter is called
                                    // ABOVE and only its RETURN VALUE reaches
                                    // this point — that value is exactly as
                                    // instance-specific as a plain instance
                                    // attribute (it's computed FROM the
                                    // instance's own state), so caching it
                                    // here under the same "found on the type"
                                    // reasoning is the identical bug in a
                                    // different disguise: every instance of
                                    // the class sharing this one cache slot
                                    // got back the FIRST instance's computed
                                    // value forever after. Confirmed via a
                                    // minimal, `__slots__`-free repro: `class
                                    // Foo: x = property(lambda self: self.v)`
                                    // — `b.x` returned `a.x`'s value.
                                    let is_property_result = {
                                        let typ_ref = typ.borrow();
                                        if let PyObject::Type {
                                            dict: type_dict,
                                            mro,
                                            ..
                                        } = &*typ_ref
                                        {
                                            let found_val: Option<PyObjectRef> =
                                                type_dict.get_str(&name).cloned().or_else(|| {
                                                    mro.iter().skip(1).find_map(|base| {
                                                        if let PyObject::Type {
                                                            dict: base_dict,
                                                            ..
                                                        } = &*base.borrow()
                                                        {
                                                            base_dict.get_str(&name).cloned()
                                                        } else {
                                                            None
                                                        }
                                                    })
                                                });
                                            found_val
                                                .map(|v| {
                                                    matches!(&*v.borrow(), PyObject::Property(_))
                                                })
                                                .unwrap_or(false)
                                        } else {
                                            false
                                        }
                                    };
                                    // A method bound to THIS instance's own
                                    // native backing (deque subclass: `pop`/
                                    // `append`/... resolved via the native
                                    // delegation at `get_attribute_impl`) is
                                    // inherently per-instance — caching it
                                    // under a `(name_idx, type_tag)` key with
                                    // no per-instance component means the NEXT
                                    // instance of the same class in this frame
                                    // silently reuses a method still bound to
                                    // the FIRST instance's backing and mutates
                                    // the wrong object (confirmed via a deque
                                    // subclass's `d.pop(); e.pop()` in one
                                    // frame). `PyObject::Closure` values are
                                    // excluded for the same reason: a
                                    // per-instance closure (e.g. a deque
                                    // subclass's `__copy__`, which captures
                                    // that instance's own backing) must not
                                    // leak into a cache another instance
                                    // reuses.
                                    let is_native_backing_bound = matches!(&*val.borrow(), PyObject::BuiltinMethod { self_obj, .. }
                                        if !matches!(&*self_obj.borrow(), PyObject::Instance { .. } | PyObject::None))
                                        || matches!(&*val.borrow(), PyObject::Closure(_));
                                    if !dict.contains_key(&name)
                                        && !is_property_result
                                        && !is_native_backing_bound
                                        && name_idx < self.frames[fi].attr_cache.len()
                                    {
                                        self.frames[fi].attr_cache[name_idx] =
                                            Some((type_tag, val.clone()));
                                    }
                                    Ok(val)
                                }
                                None => {
                                    // Check for __getattr__ before erroring —
                                    // via the full mro, not just typ's own
                                    // dict: `__getattr__` is very commonly
                                    // defined on a BASE class (e.g. Django's
                                    // `LazyObject.__getattr__ =
                                    // new_method_proxy(getattr)`, inherited
                                    // by `SimpleLazyObject`) rather than
                                    // redeclared on every subclass, and the
                                    // instance's own exact class rarely
                                    // defines it directly.
                                    let getattr_method =
                                        crate::object::lookup_dunder_via_mro(typ, "__getattr__");
                                    if let Some(getattr_method) = getattr_method {
                                        self.call_function(
                                            getattr_method,
                                            vec![obj.clone(), py_str(&name)],
                                            vec![],
                                        )
                                    } else if name == "__doc__" {
                                        // Every real object has __doc__ (defaults to
                                        // None) — see the matching fallback in
                                        // object.rs's ObjectAccess::get_attribute.
                                        Ok(py_none())
                                    } else {
                                        // Attach name/obj to the AttributeError
                                        // (CPython: `exc.name`/`exc.obj` after
                                        // `obj.missing_attr`).
                                        let mut extra = std::collections::HashMap::new();
                                        extra.insert("name".to_string(), py_str(&name));
                                        extra.insert("obj".to_string(), obj.clone());
                                        Err(PyError::Exception(
                                            "AttributeError".to_string(),
                                            PyObjectRef::new(PyObject::Exception {
                                                typ: "AttributeError".to_string(),
                                                args: vec![py_str(&format!(
                                                    "'{}' object has no attribute '{}'",
                                                    crate::object::get_type_name_for_instance(typ),
                                                    name
                                                ))],
                                                cause: None,
                                                suppress_context: false,
                                                context: None,
                                                traceback: None,
                                                extra: Some(extra),
                                            }),
                                        ))
                                    }
                                }
                            }
                        }
                        _ => {
                            let type_name = obj_borrowed.type_name();
                            // Check inline cache first — but NOT for a TYPE
                            // receiver: the cache is keyed only by
                            // `(type_name, name)` and is populated by
                            // VALUE-level lookups (`line.strip` on a str
                            // value caches the instance strip under
                            // ("str","strip")), so a later TYPE-level lookup
                            // (`str.strip` on the str CLASS) would hit it and
                            // return the WRONG, already-value-bound method
                            // bound to the class (str.strip(s) then passed
                            // the class as self -> "<class 'str'>"). Type
                            // attribute access is rare and needs the full
                            // get_attribute path (which returns the raw
                            // unbound descriptor).
                            let is_type_obj = matches!(&*obj_borrowed, PyObject::Type { .. });
                            let is_globals = matches!(&*obj_borrowed, PyObject::Globals(_));
                            let cached = if is_type_obj || is_globals {
                                None
                            } else {
                                crate::vm::ATTR_CACHE.with(|c| {
                                    c.borrow().get(&(type_name.clone(), name.clone())).copied()
                                })
                            };
                            if let Some(func) = cached {
                                drop(obj_borrowed);
                                Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                                    name: name.clone(),
                                    func,
                                    self_obj: obj.clone(),
                                }))
                            } else {
                                let direct = obj_borrowed.get_attribute(&name);
                                let obj_type_name_for_err = obj_borrowed.type_name();
                                let attr = match direct {
                                    Ok(v) => v,
                                    Err(_) => {
                                        // Metaclass attribute fallback: a
                                        // class-level attribute not found on
                                        // the class's own dict/mro may still
                                        // exist on its *metaclass* (e.g. a
                                        // `@property` defined on a custom
                                        // metaclass like Django's
                                        // `ChoicesType.choices` — meant to be
                                        // read as `SomeChoicesClass.choices`,
                                        // with the class itself as the
                                        // property's "self"). Ordinary
                                        // classes have no METATYPE_KEY set,
                                        // so this is a no-op for them.
                                        let metatype_hit = if is_type_obj {
                                            crate::object::metatype_of(&obj).and_then(|mt| {
                                                if let PyObject::Type { mro, .. } = &*mt.borrow() {
                                                    for base in mro.iter() {
                                                        if let PyObject::Type { dict, .. } =
                                                            &*base.borrow()
                                                        {
                                                            if let Some(val) = dict.get_str(&name) {
                                                                return Some(val.clone());
                                                            }
                                                        }
                                                    }
                                                }
                                                None
                                            })
                                        } else {
                                            None
                                        };
                                        match metatype_hit {
                                            Some(val) => {
                                                let is_property = if let PyObject::Property(ref d) =
                                                    &*val.borrow()
                                                {
                                                    d.getter.is_some()
                                                } else {
                                                    false
                                                };
                                                if is_property {
                                                    let getter = if let PyObject::Property(ref d) =
                                                        &*val.borrow()
                                                    {
                                                        d.getter.clone().unwrap()
                                                    } else {
                                                        unreachable!()
                                                    };
                                                    drop(obj_borrowed);
                                                    let result = self.call_function(
                                                        getter,
                                                        vec![obj.clone()],
                                                        vec![],
                                                    )?;
                                                    self.frames[fi].push(result);
                                                    return Ok(true);
                                                }
                                                // `obj` (a class) is being accessed as an
                                                // INSTANCE of its own metaclass here (that's
                                                // what "found on the metatype, not on obj's
                                                // own dict/mro" means) — an ordinary method
                                                // found this way must auto-bind `self=obj`,
                                                // exactly like any instance accessing a
                                                // regular method, or its first real
                                                // parameter (e.g. Django's metaclass method
                                                // `add_to_class(cls, name, value)`, called as
                                                // `new_class.add_to_class(name, value)`) never
                                                // gets bound and every later positional arg
                                                // silently shifts left by one. This is
                                                // distinct from ordinary `SomeClass.method`
                                                // access (the `is_function => Ok(attr)` case
                                                // below), which correctly stays unbound.
                                                if matches!(&*val.borrow(), PyObject::Function(_)) {
                                                    drop(obj_borrowed);
                                                    self.frames[fi].push(PyObjectRef::imm(
                                                        PyObject::BoundMethod {
                                                            func: val,
                                                            self_obj: obj.clone(),
                                                        },
                                                    ));
                                                    return Ok(true);
                                                }
                                                val
                                            }
                                            None => {
                                                drop(obj_borrowed);
                                                if name == "__doc__" {
                                                    self.frames[fi].push(py_none());
                                                    return Ok(true);
                                                }
                                                // PEP 562 module `__getattr__`:
                                                // a module whose own dict lacks
                                                // the attribute but defines
                                                // `__getattr__` (lazy-attribute
                                                // pattern; real trigger: the
                                                // vendored `test.support.
                                                // _hypothesis_stubs.strategies`,
                                                // which backends every
                                                // strategy name through a
                                                // module-level `__getattr__`)
                                                // must have it invoked before
                                                // erroring.
                                                if matches!(&*obj.borrow(), PyObject::Module { .. })
                                                {
                                                    let g =
                                                        obj.borrow().get_attribute("__getattr__");
                                                    if let Ok(getattr_method) = g {
                                                        if !matches!(
                                                            &*getattr_method.borrow(),
                                                            PyObject::None
                                                        ) {
                                                            // Module __getattr__ takes ONLY the attribute name —
                                                            // the module itself is NOT bound as self.
                                                            let result = self.call_function(
                                                                getattr_method,
                                                                vec![py_str(&name)],
                                                                vec![],
                                                            )?;
                                                            self.frames[fi].push(result);
                                                            return Ok(true);
                                                        }
                                                    }
                                                }
                                                return Err(PyError::attribute_error(format!(
                                                    "'{}' object has no attribute '{}'",
                                                    obj_type_name_for_err, name
                                                )));
                                            }
                                        }
                                    }
                                };
                                drop(obj_borrowed);
                                // Generic descriptor protocol for class-level
                                // attribute access (`Foo.attr`, `obj` here is
                                // the type itself): a plain user-defined
                                // descriptor class (any Instance whose type
                                // defines __get__ — e.g. Django's
                                // class_or_instance_method) must have __get__
                                // invoked with instance=None, matching the
                                // generic __get__ handling already done for
                                // instance-level access above. Builtin
                                // Property/StaticMethod/ClassMethod/Function
                                // descriptors are already special-cased below
                                // and are never PyObject::Instance, so this
                                // can't double-invoke them.
                                if is_type_obj {
                                    if matches!(&*attr.borrow(), PyObject::Instance { .. }) {
                                        if let Ok(get_fn) = attr.borrow().get_attribute("__get__") {
                                            if std::env::var("RPY_DEBUG_DESCRIPTOR2").is_ok() {
                                                eprintln!("CLASS-LEVEL __get__: attr_name={} obj_type={:?}", name, obj.borrow().type_name());
                                            }
                                            let result = self.call_function(
                                                get_fn,
                                                vec![attr.clone(), py_none(), obj.clone()],
                                                vec![],
                                            )?;
                                            self.frames[fi].push(result);
                                            return Ok(true);
                                        }
                                    }
                                }
                                // Resolve classmethod descriptor for type attribute access
                                {
                                    let ab = attr.borrow();
                                    if let PyObject::ClassMethod { func } = &*ab {
                                        let func_clone = func.clone();
                                        let cls_obj = obj.clone();
                                        drop(ab);
                                        let bound = PyObjectRef::new(PyObject::BoundMethod {
                                            func: func_clone,
                                            self_obj: cls_obj,
                                        });
                                        self.frames[fi].push(bound);
                                        return Ok(true);
                                    }
                                }
                                // Only rebind self_obj (and cache the
                                // func-pointer fast path keyed on it) when
                                // the found `BuiltinMethod`'s own self_obj is
                                // still the `PyObject::None` PLACEHOLDER —
                                // the established convention native
                                // container methods (File/List/Dict/Set/
                                // frozenset's own `.append`/`.get`/etc.) use,
                                // meaning "rebind me to whatever object I
                                // was actually looked up on". A
                                // BuiltinMethod that's already bound to some
                                // OTHER real object (e.g. a MODULE-level
                                // `iskeyword = frozenset(kwlist).__contains__`
                                // — self_obj is that frozenset, permanently,
                                // and `obj` here is the *module* being
                                // accessed as `keyword.iskeyword`) must be
                                // returned completely unchanged. Previously
                                // this unconditionally rebuilt EVERY
                                // BuiltinMethod found this way with
                                // `self_obj: obj.clone()`, discarding the
                                // real target and substituting the
                                // currently-accessed object instead —
                                // confirmed general via `import keyword;
                                // keyword.iskeyword("if")` raising
                                // `RuntimeError: __contains__ on
                                // non-frozenset` (self_obj had silently
                                // become the `keyword` module itself).
                                let is_placeholder_self = matches!(&*attr.borrow(), PyObject::BuiltinMethod { self_obj, .. } if matches!(&*self_obj.borrow(), PyObject::None));
                                let is_function = matches!(&*attr.borrow(), PyObject::Function(_));
                                if is_placeholder_self {
                                    let (n, func) = {
                                        let b = attr.borrow();
                                        if let PyObject::BuiltinMethod { name: n, func, .. } = &*b {
                                            (n.clone(), *func)
                                        } else {
                                            unreachable!()
                                        }
                                    };
                                    // Cache for next time — but NOT
                                    // `__init__` (nor `__new__`): a native
                                    // VALUE's `__init__` (e.g. a raw deque's,
                                    // resolved via `attrs.rs`'s per-value arm)
                                    // and the same-name TYPE-level attribute
                                    // (`deque.__init__`, the native-base
                                    // initializer) are DIFFERENT methods, yet
                                    // this cache is keyed only by
                                    // `(type_name, name)` — caching the
                                    // value-level one made `deque.__init__`
                                    // silently return the wrong function after
                                    // any `d.__init__(...)` call.
                                    if n != "__init__"
                                        && n != "__new__"
                                        && !matches!(&*obj.borrow(), PyObject::Globals(_))
                                    {
                                        crate::vm::ATTR_CACHE.with(|c| {
                                            c.borrow_mut()
                                                .insert((type_name.clone(), n.clone()), func);
                                        });
                                    }
                                    Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                                        name: n,
                                        func,
                                        self_obj: obj.clone(),
                                    }))
                                } else if is_function {
                                    Ok(attr)
                                } else {
                                    Ok(attr)
                                }
                            }
                        }
                    }
                }?;
                self.frames[fi].push(result);
            }

            _ => return Ok(false),
        }
        Ok(true)
    }
}
