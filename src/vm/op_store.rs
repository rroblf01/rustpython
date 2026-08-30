use crate::bytecode::Opcode;
use crate::object::*;
use crate::vm::VirtualMachine;

impl VirtualMachine {
    pub(crate) fn handle_store(&mut self, fi: usize, op: Opcode, arg: u32) -> PyResult<bool> {
        match op {
            Opcode::STORE_ATTR => {
                let name_idx = arg as usize;
                let name = crate::interner::lookup(self.frames[fi].code.names[name_idx]);
                let val = self.frames[fi].pop()?;
                let obj = self.frames[fi].pop()?;
                if std::env::var("RPY_DEBUG_ATTR").is_ok() {
                    let kind = match &*obj.borrow() {
                        PyObject::Type { name: n, .. } => format!("Type({})", n),
                        PyObject::Module { name: n, .. } => format!("Module({})", n),
                        PyObject::Instance { .. } => "Instance".to_string(),
                        other => format!("{:?}", std::mem::discriminant(other)),
                    };
                    eprintln!("STORE_ATTR: name={} obj_kind={}", name, kind);
                }

                // Check for __setattr__ on Instance types first — walks the
                // MRO (not just the leaf type's own dict), so a `__setattr__`
                // defined on a BASE class (e.g. `unittest.mock.NonCallableMock`,
                // inherited by `MagicMock`/`Mock`/every other mock subclass
                // that never redefines it) is actually found instead of
                // silently falling through to plain instance-dict assignment.
                {
                    let obj_borrowed = obj.borrow();
                    if let PyObject::Instance { typ, .. } = &*obj_borrowed {
                        let typ_clone = typ.clone();
                        drop(obj_borrowed);
                        if let Some(setattr_method) =
                            crate::object::lookup_dunder_via_mro(&typ_clone, "__setattr__")
                        {
                            // Call __setattr__ for side effects (validation, clearing caches)
                            let result = self.call_function(
                                setattr_method,
                                vec![obj.clone(), py_str(&name), val.clone()],
                                vec![],
                            );
                            // Also set the attribute directly in the instance dict, since
                            // __dict__ returns a COPY and self.__dict__[key] = value inside
                            // __setattr__ would modify the copy, not the original.
                            if let PyObject::Instance { dict, .. } = &mut *obj.borrow_mut() {
                                dict.insert_str(&name, val.clone());
                            }
                            result?;
                            return Ok(true);
                        }
                    }
                }

                // Check for __set__ descriptor protocol on Instance types
                let descriptor_clone = {
                    let obj_borrowed = obj.borrow();
                    if let PyObject::Instance { typ, .. } = &*obj_borrowed {
                        let typ_ref = typ.borrow();
                        if let PyObject::Type {
                            dict: type_dict, ..
                        } = &*typ_ref
                        {
                            type_dict.get_str(&name).cloned()
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                };
                if let Some(descriptor) = descriptor_clone {
                    // Property is special-cased directly (matching how LOAD_ATTR's
                    // getter path already calls `self.call_function` on the real
                    // getter function directly, not through a wrapper) instead of
                    // going through the generic `get_attribute("__set__")` below.
                    // That generic path returns a `BuiltinMethod` whose closure
                    // body calls the free `call_bound_method` function internally
                    // (a plain `fn(&[PyObjectRef])` has no `&mut VirtualMachine`
                    // to call through) — which spins up a disposable VM with an
                    // empty module registry. A property setter that does a lazy
                    // `import` internally (a real, common Django pattern used
                    // specifically to sidestep circular imports) would then
                    // re-import everything from scratch in that disposable VM
                    // instead of seeing what's already loaded.
                    let property_setter = {
                        let d = descriptor.borrow();
                        if let PyObject::Property(ref data) = &*d {
                            data.setter.clone()
                        } else {
                            None
                        }
                    };
                    if let Some(setter_fn) = property_setter {
                        self.call_function(setter_fn, vec![obj.clone(), val.clone()], vec![])?;
                        return Ok(true);
                    }
                    let setter_method = { descriptor.borrow().get_attribute("__set__").ok() };
                    if let Some(setter_method) = setter_method {
                        let result = self.call_function(
                            setter_method,
                            vec![descriptor, obj.clone(), val.clone()],
                            vec![],
                        );
                        match result {
                            Ok(_) => return Ok(true),
                            Err(e) => return Err(e),
                        }
                    } else {
                        // Descriptor exists but no __set__ (non-data descriptor), fall through
                    }
                }
                // `obj.borrow_mut()` panics unconditionally for any
                // non-`Mut`-wrapped value (SmallInt/SmallBool/SmallFloat/
                // SmallStr/None, or an `Imm`-wrapped Tuple/Bytes/Function/
                // Type/Code/boxed-Int/Str/Float) — genuinely attribute-
                // settable things (Instance, Type, Module, Exception) are
                // ALWAYS `Mut` in this codebase, so anything reaching here
                // that ISN'T `Mut` is a real attempt to set an attribute on
                // an immutable/inline value (`(5).x = 1`, `"s".x = 1`,
                // `(1, 2).x = 1`) — real CPython raises a plain
                // `AttributeError` there, not a process-ending crash. This
                // was one of the highest-impact bugs found this session:
                // it crashed the WHOLE interpreter process (not just the
                // current statement) for something this common — including
                // every test file that deliberately checks this raises via
                // `self.assertRaises(AttributeError, setattr, x, 'attr', v)`.
                if let PyObject::WeakProxy { target, .. } = &*obj.borrow() {
                    if let Some(rc) = target.upgrade() {
                        rc.borrow_mut().set_attribute(&name, val)?;
                        return Ok(true);
                    } else {
                        return Err(PyError::reference_error("weakly-referenced object no longer exists"));
                    }
                }
                if !matches!(&obj, PyObjectRef::Mut(_)) {
                    return Err(PyError::attribute_error(format!(
                        "'{}' object has no attribute '{}'",
                        obj.borrow().type_name(),
                        name
                    )));
                }
                obj.borrow_mut().set_attribute(&name, val)?;
            }

            Opcode::STORE_SUBSCR => {
                let val = self.frames[fi].pop()?;
                let index = self.frames[fi].pop()?;
                let obj = self.frames[fi].pop()?;
                // If `obj` is an Instance with a Python-defined __setitem__,
                // call it via `self.call_function` (the real, already-live
                // VM) rather than falling into the free `py_setitem`
                // function's own Instance-dispatch, which calls it via
                // `call_bound_method` — a separate, pre-existing, documented
                // limitation that spins up a brand-new disposable
                // `VirtualMachine::new()` for the call. That's merely
                // wasteful for most code, but genuinely catastrophic for
                // any dict-subclass with a custom `__setitem__` used during
                // this VM's own construction (e.g. enum's `_EnumDict`,
                // whose `EnumType.__new__` does `namespace[key] = ...`) —
                // the disposable VM's construction re-runs the same
                // stdlib bootstrap, hits the same assignment again, and
                // recurses without end (confirmed via gdb backtrace).
                // Falls back to the free function for everything else
                // (native list/dict/tuple assignment, or an Instance with
                // no override delegating to its native backing), which
                // needs no VM access at all.
                let setitem_fn = if let PyObject::Instance { typ, .. } = &*obj.borrow() {
                    crate::object::lookup_dunder_via_mro(typ, "__setitem__")
                } else {
                    None
                };
                if let Some(f) = setitem_fn {
                    self.call_function(f, vec![obj.clone(), index, val], vec![])?;
                } else {
                    py_setitem(&obj, &index, val)?;
                }
            }

            Opcode::DELETE_SUBSCR => {
                let index = self.frames[fi].pop()?;
                let obj = self.frames[fi].pop()?;
                py_delitem(&obj, &index)?;
            }

            Opcode::DELETE_ATTR => {
                let name_idx = arg as usize;
                let name = crate::interner::lookup(self.frames[fi].code.names[name_idx]);
                let obj = self.frames[fi].pop()?;
                // Check for __delattr__ on Instance types first — walks the
                // MRO like the matching STORE_ATTR check above, so an
                // inherited `__delattr__` (e.g. `unittest.mock.NonCallableMock`'s,
                // used by every mock subclass that never redefines it) is
                // actually found instead of silently falling through to the
                // generic instance-attribute-deletion path below.
                {
                    let obj_borrowed = obj.borrow();
                    if let PyObject::Instance { typ, .. } = &*obj_borrowed {
                        let typ_clone = typ.clone();
                        drop(obj_borrowed);
                        if let Some(delattr_method) =
                            crate::object::lookup_dunder_via_mro(&typ_clone, "__delattr__")
                        {
                            self.call_function(
                                delattr_method,
                                vec![obj.clone(), py_str(&name)],
                                vec![],
                            )?;
                            return Ok(true);
                        }
                    }
                }
                // Check for __delete__ descriptor protocol
                let descriptor = {
                    let obj_borrowed = obj.borrow();
                    if let PyObject::Instance { typ, .. } = &*obj_borrowed {
                        let typ_ref = typ.borrow();
                        if let PyObject::Type {
                            dict: type_dict, ..
                        } = &*typ_ref
                        {
                            type_dict.get_str(&name).cloned()
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                };
                if let Some(ref desc) = descriptor {
                    if let Ok(deleter) = desc.borrow().get_attribute("__delete__") {
                        let result =
                            self.call_function(deleter, vec![desc.clone(), obj.clone()], vec![]);
                        match result {
                            Ok(_) => return Ok(true),
                            Err(e) => return Err(e),
                        }
                    }
                }
                // `.borrow_mut()` panics unconditionally for anything that
                // ISN'T `PyObjectRef::Mut` — every inline variant plus any
                // `Imm`-wrapped value (boxed Int, Range, Tuple, Str, ...).
                // `del some_immutable_value.attr` (real trigger: CPython's
                // own `test_range.py`, `del rangeobj.start` — a `range`
                // object's `start`/`stop`/`step` are read-only, expected to
                // raise a clean `AttributeError`) previously panicked the
                // whole process instead. Same fix shape as `builtin_setattr`
                // already applies for `setattr()`.
                if let PyObject::WeakProxy { target, .. } = &*obj.borrow() {
                    if let Some(rc) = target.upgrade() {
                        rc.borrow_mut().del_attribute(&name)?;
                        return Ok(true);
                    } else {
                        return Err(PyError::reference_error("weakly-referenced object no longer exists"));
                    }
                }
                if !matches!(obj, PyObjectRef::Mut(_)) {
                    return Err(PyError::attribute_error(format!(
                        "'{}' object attribute '{}' is read-only",
                        obj.borrow().type_name(),
                        name
                    )));
                }
                obj.borrow_mut().del_attribute(&name)?;
            }

            _ => return Ok(false),
        }
        Ok(true)
    }
}
