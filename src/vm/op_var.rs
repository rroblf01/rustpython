use crate::bytecode::Opcode;
use crate::interner::{self};
use crate::object::*;
use crate::vm::VirtualMachine;

impl VirtualMachine {
    /// Write through to a custom `__prepare__` namespace (class-body
    /// frames only; see `Frame::prepared_namespace`). Plain `Dict`
    /// namespaces take a direct-set fast path (identical semantics to
    /// the frame-globals write, just observable); anything else goes
    /// through its real `__setitem__` looked up on the live VM — never a
    /// disposable one (class bodies can execute during interpreter
    /// bootstrap, where a fresh VM would recurse; same hazard documented
    /// on `STORE_SUBSCR`).
    pub(crate) fn class_ns_set(
        &mut self,
        prepared: &PyObjectRef,
        name: &str,
        val: PyObjectRef,
    ) -> PyResult<()> {
        let key = py_str(name);
        // `prepared` is whatever a custom `__prepare__` returned — it need
        // not be a heap-allocated mutable (`Mut`) object at all (a bogus
        // `__prepare__` can hand back an int, a string, `None`, ...), so use
        // the fallible `try_borrow_mut()` here rather than `borrow_mut()`:
        // the latter panics outright on a non-`Mut` value instead of falling
        // through to the `__setitem__`/native-backing/TypeError paths below.
        if let Ok(mut guard) = prepared.try_borrow_mut() {
            if let PyObject::Dict(d) = &mut *guard {
                d.set(key, val)?;
                return Ok(());
            }
        }
        let setitem_fn = if let PyObject::Instance { typ, .. } = &*prepared.borrow() {
            crate::object::lookup_dunder_via_mro(typ, "__setitem__")
        } else {
            None
        };
        if let Some(f) = setitem_fn {
            self.call_function(f, vec![prepared.clone(), key, val], vec![])?;
            return Ok(());
        }
        if let Some(native) = crate::object::native_backing_of(prepared) {
            if let PyObject::Dict(d) = &mut *native.borrow_mut() {
                d.set(key, val)?;
                return Ok(());
            }
        }
        Err(PyError::type_error(format!(
            "'{}' object is not a mapping (missing __setitem__)",
            prepared.borrow().type_name()
        )))
    }

    /// Read through a custom `__prepare__` namespace. Returns `Ok(None)`
    /// on a miss (absent key) so the caller falls through to the rest of
    /// the LOAD_NAME chain exactly like a frame-globals miss; any other
    /// error propagates.
    fn class_ns_get(
        &mut self,
        prepared: &PyObjectRef,
        name: &str,
    ) -> PyResult<Option<PyObjectRef>> {
        let key = py_str(name);
        if let PyObject::Dict(d) = &*prepared.borrow() {
            return d.get(&key);
        }
        if let PyObject::Instance { typ, .. } = &*prepared.borrow() {
            if let Some(f) = crate::object::lookup_dunder_via_mro(typ, "__getitem__") {
                match self.call_function(f, vec![prepared.clone(), key], vec![]) {
                    Ok(v) => return Ok(Some(v)),
                    Err(PyError::KeyError(_)) => return Ok(None),
                    Err(e) => return Err(e),
                }
            }
            if let Some(native) = crate::object::native_backing_of(prepared) {
                if let PyObject::Dict(d) = &*native.borrow() {
                    return d.get(&key);
                }
            }
            return Ok(None);
        }
        Ok(None)
    }

    /// Delete through a custom `__prepare__` namespace. Mirrors the
    /// existing DELETE_NAME leniency (missing names are silently
    /// ignored); any other error propagates.
    fn class_ns_del(&mut self, prepared: &PyObjectRef, name: &str) -> PyResult<()> {
        let key = py_str(name);
        if let PyObject::Dict(d) = &mut *prepared.borrow_mut() {
            let _ = d.remove(&key);
            return Ok(());
        }
        if let PyObject::Instance { typ, .. } = &*prepared.borrow() {
            if let Some(f) = crate::object::lookup_dunder_via_mro(typ, "__delitem__") {
                match self.call_function(f, vec![prepared.clone(), key], vec![]) {
                    Ok(_) => return Ok(()),
                    Err(PyError::KeyError(_)) => return Ok(()),
                    Err(e) => return Err(e),
                }
            }
            if let Some(native) = crate::object::native_backing_of(prepared) {
                if let PyObject::Dict(d) = &mut *native.borrow_mut() {
                    let _ = d.remove(&key);
                    return Ok(());
                }
            }
            return Ok(());
        }
        Ok(())
    }

    pub(crate) fn handle_var(&mut self, fi: usize, op: Opcode, arg: u32) -> PyResult<bool> {
        match op {
            Opcode::NOP => {
                return Ok(true);
            }

            Opcode::LOAD_CONST => {
                let const_idx = arg as usize;
                let cached = self.frames[fi]
                    .code
                    .const_cache
                    .borrow()
                    .get(const_idx)
                    .and_then(|c| c.clone());
                let obj = if let Some(obj) = cached {
                    obj
                } else {
                    let const_val = self.frames[fi]
                        .code
                        .consts
                        .get(const_idx)
                        .ok_or_else(|| {
                            PyError::runtime_error(format!(
                                "constant index out of range: {}",
                                const_idx
                            ))
                        })?
                        .clone();
                    let obj = crate::vm::helpers::eval_const_value(const_val)?;
                    let mut cache = self.frames[fi].code.const_cache.borrow_mut();
                    if cache.len() <= const_idx {
                        cache.resize(const_idx + 1, None);
                    }
                    cache[const_idx] = Some(obj.clone());
                    obj
                };
                self.frames[fi].push(obj);
            }

            Opcode::LOAD_NAME => {
                let name_idx = arg as usize;
                let name = crate::interner::lookup_str(self.frames[fi].code.names[name_idx]);
                let val = {
                    let f = &self.frames[self.frames.len() - 1];
                    f.get_local(name)
                        .cloned()
                        .or_else(|| {
                            let fv_idx = f.code.freevars.iter().position(|n| n == name)?;
                            let cell = f.closure.get(fv_idx)?;
                            match &*cell.borrow() {
                                PyObject::Cell { value: Some(inner) } => Some(inner.clone()),
                                PyObject::Cell { value: None } => None,
                                _ => Some(cell.clone()),
                            }
                        })
                        // Live exec-locals mapping (`exec(code, globals,
                        // locals)` with distinct locals) shadows frame
                        // globals, like CPython's f_locals.
                        .or_else(|| crate::vm::helpers::frame_exec_local_get(f, name))
                        .or_else(|| f.globals.borrow().get(&interner::intern(name)).cloned())
                };
                // A custom `__prepare__` namespace IS the class body's
                // locals: names it pre-populated (before any body store)
                // must resolve here, shadowed by real body stores above
                // (which live in frame.globals too via dual-write) but
                // ahead of module globals/builtins.
                let val = match val {
                    Some(v) => Some(v),
                    None => {
                        if let Some(prep) = self.frames[fi].prepared_namespace.clone() {
                            self.class_ns_get(&prep, name)?
                        } else {
                            None
                        }
                    }
                };
                let val = match val {
                    Some(v) => Some(v),
                    None => {
                        let f = &self.frames[self.frames.len() - 1];
                        f.module_globals
                            .as_ref()
                            .and_then(|mg| mg.borrow().get(&interner::intern(name)).cloned())
                            .or_else(|| f.builtins.get(&interner::intern(name)).cloned())
                    }
                };
                match val {
                    Some(v) => self.frames[fi].push(v),
                    None => return Err(PyError::name_error_for(name)),
                }
            }

            Opcode::STORE_NAME => {
                let name_idx = arg as usize;
                let name = crate::interner::lookup(self.frames[fi].code.names[name_idx]);
                let val = self.frames[fi].pop()?;
                // Live exec-locals mapping (`exec(code, globals, locals)`
                // with distinct locals): stores land in it directly and
                // nowhere else — like CPython writing f_locals. Skips the
                // module/class mirroring below, which is meaningless (and
                // leaky) for a throwaway exec namespace.
                if crate::vm::helpers::frame_exec_local_set(&self.frames[fi], &name, val.clone()) {
                    if self.frames[fi].frame_locals_obj.is_some() {
                        self.sync_frame_locals(fi);
                    }
                    return Ok(true);
                }
                if let Some(order) = self.frames[fi].name_order.clone() {
                    let mut order = order.borrow_mut();
                    if !order.contains(&name) {
                        order.push(name.clone());
                    }
                }
                if let Some(live_module) = self.frames[fi].live_module.clone() {
                    if let PyObject::Module { dict, .. } = &mut *live_module.borrow_mut() {
                        dict.insert_str(&name, val.clone());
                    }
                }
                let sid = interner::intern(&name);
                self.frames[fi]
                    .globals
                    .borrow_mut()
                    .insert(sid, val.clone());
                // Dual-write into a custom `__prepare__` namespace (see
                // `Frame::prepared_namespace`): every body store must be
                // observable there one call at a time.
                if let Some(prep) = self.frames[fi].prepared_namespace.clone() {
                    self.class_ns_set(&prep, &name, val.clone())?;
                }
                if self.frames[fi].name_order.is_none() {
                    let mod_name_opt = self.frames[fi]
                        .globals
                        .borrow()
                        .get(&interner::intern("__name__"))
                        .cloned();
                    if let Some(mod_name_ref) = mod_name_opt {
                        if let PyObject::Str(s) = &*mod_name_ref.borrow() {
                            if let Some(mod_ref) = self.modules.get(s.as_str()).cloned() {
                                if let PyObject::Module { dict, .. } = &mut *mod_ref.borrow_mut() {
                                    dict.insert(sid, val.clone());
                                }
                            }
                        }
                    }
                    if let Some(mg) = self.frames[fi].module_globals.clone() {
                        mg.borrow_mut().insert(sid, val);
                    } else if self.frames[fi].globals.borrow().contains_key(&sid) {
                    }
                }
                if self.frames[fi].frame_locals_obj.is_some() {
                    self.sync_frame_locals(fi);
                }
            }

            Opcode::LOAD_FAST => {
                let var_idx = arg as usize;
                let val = {
                    let f = &self.frames[self.frames.len() - 1];
                    f.fast_locals.get(var_idx).and_then(|v| v.clone())
                };
                match val {
                    Some(v) if matches!(&*v.borrow(), PyObject::Cell { .. }) => {
                        let inner = match &*v.borrow() {
                            PyObject::Cell { value: Some(inner) } => Some(inner.clone()),
                            PyObject::Cell { value: None } => None,
                            _ => unreachable!(),
                        };
                        match inner {
                            Some(inner) => self.frames[fi].push(inner),
                            None => return Err(PyError::unbound_local_error(format!("cannot access local variable '{}' where it is not associated with a value",
                                self.frames[fi].code.varnames.get(var_idx).map_or("?", |&s| crate::interner::lookup_str(s))))),
                        }
                    }
                    Some(v) => self.frames[fi].push(v),
                    None => {
                        if std::env::var("RPY_DEBUG_NAMEERROR").is_ok() {
                            eprintln!(
                                "LOAD_FAST unbound: func={} file={} line={:?} varnames={:?}",
                                self.frames[fi].code.name,
                                self.frames[fi].code.filename,
                                self.frames[fi]
                                    .code
                                    .line_number(self.frames[fi].ip.saturating_sub(1)),
                                self.frames[fi].code.varnames
                            );
                        }
                        return Err(PyError::unbound_local_error(format!("cannot access local variable '{}' where it is not associated with a value",
                            self.frames[fi].code.varnames.get(var_idx).map_or("?", |&s| crate::interner::lookup_str(s)))));
                    }
                }
            }

            Opcode::STORE_FAST => {
                let var_idx = arg as usize;
                let val = self.frames[fi].pop()?;
                let frame = &mut self.frames[fi];
                if var_idx < frame.fast_locals.len() {
                    let is_existing_cell = matches!(&frame.fast_locals[var_idx], Some(existing) if matches!(&*existing.borrow(), PyObject::Cell { .. }));
                    if is_existing_cell {
                        if let Some(existing) = frame.fast_locals[var_idx].clone() {
                            if let PyObject::Cell { value } = &mut *existing.borrow_mut() {
                                *value = Some(val.clone());
                            }
                        }
                    } else {
                        frame.fast_locals[var_idx] = Some(val.clone());
                    }
                }
                // Fast path: skip HashMap insert unless frame locals are
                // actually observed (via `locals()` / frame object). LOAD_FAST
                // reads directly from fast_locals, so keeping the map in sync
                // on every store is pure overhead for tight loops (quickbench
                // b_arith etc pay a hash insert per STORE_FAST otherwise).
                if frame.frame_locals_obj.is_some() {
                    let name = crate::interner::lookup_str(frame.code.varnames[var_idx]);
                    frame.insert_local(name, val);
                    self.sync_frame_locals(fi);
                }
            }

            Opcode::LOAD_GLOBAL => {
                let instr_ip = self.frames[fi].ip - 1;
                let mut cached_from_builtins = false;
                if let Some(cached) = self.frames[fi]
                    .global_cache
                    .get(instr_ip)
                    .and_then(|c| c.clone())
                {
                    self.frames[fi].push(cached);
                } else {
                    let name_idx = arg as usize;
                    // `code.names` already holds the interned StrId — use it
                    // directly for every dict probe instead of round-tripping
                    // through `lookup_str` + re-`intern` (a hash-table op per
                    // probe, up to 4 per miss). The `&str` is only materialized
                    // for the cold NameError path.
                    let sid = self.frames[fi].code.names[name_idx];
                    let mut val = None;
                    {
                        let f = &self.frames[self.frames.len() - 1];
                        if let Some(v) = f.globals.borrow().get(&sid).cloned() {
                            val = Some(v);
                        } else if let Some(v) = f
                            .module_globals
                            .as_ref()
                            .and_then(|mg| mg.borrow().get(&sid).cloned())
                        {
                            val = Some(v);
                        } else {
                            let builtins_mod = f
                                .globals
                                .borrow()
                                .get(&interner::intern("__builtins__"))
                                .cloned()
                                .or_else(|| {
                                    f.module_globals.as_ref().and_then(|mg| {
                                        mg.borrow().get(&interner::intern("__builtins__")).cloned()
                                    })
                                });
                            if let Some(bmod) = builtins_mod {
                                let b = bmod.borrow();
                                if let PyObject::Module { dict, .. } = &*b {
                                    if let Some(v) = dict.get(&sid) {
                                        val = Some(v.clone());
                                    }
                                }
                            }
                            if val.is_none() {
                                if let Some(v) = f.builtins.get(&sid).cloned() {
                                    val = Some(v);
                                    cached_from_builtins = true;
                                }
                            }
                        }
                    }
                    match val {
                        Some(v) => {
                            if !cached_from_builtins
                                && instr_ip < self.frames[fi].global_cache.len()
                            {
                                self.frames[fi].global_cache[instr_ip] = Some(v.clone());
                            }
                            self.frames[fi].push(v);
                        }
                        None => {
                            let name =
                                crate::interner::lookup_str(self.frames[fi].code.names[name_idx]);
                            return Err(PyError::name_error_for(name));
                        }
                    }
                }
            }

            Opcode::STORE_GLOBAL => {
                let name_idx = arg as usize;
                let sid = self.frames[fi].code.names[name_idx];
                let val = self.frames[fi].pop()?;
                self.frames[fi]
                    .globals
                    .borrow_mut()
                    .insert(sid, val.clone());
                let mod_name_opt = self.frames[fi]
                    .globals
                    .borrow()
                    .get(&interner::intern("__name__"))
                    .cloned();
                if let Some(mod_name_ref) = mod_name_opt {
                    if let PyObject::Str(s) = &*mod_name_ref.borrow() {
                        if let Some(mod_ref) = self.modules.get(s.as_str()).cloned() {
                            if let PyObject::Module { dict, .. } = &mut *mod_ref.borrow_mut() {
                                dict.insert(sid, val.clone());
                            }
                        }
                    }
                }
                if let Some(mg) = self.frames[fi].module_globals.clone() {
                    mg.borrow_mut().insert(sid, val);
                }
            }

            Opcode::LOAD_DEREF => {
                let idx = arg as usize;
                let (cell_ref, is_freevar, name_str): (Option<PyObjectRef>, bool, String) = {
                    let f = &self.frames[fi];
                    let code = &f.code;
                    if idx < code.cellvars.len() {
                        let name = &code.cellvars[idx];
                        let var_idx = code
                            .varnames
                            .iter()
                            .position(|&n| crate::interner::intern_eq(n, name))
                            .ok_or_else(|| {
                                PyError::name_error(format!("variable '{}' not found", name))
                            })?;
                        (f.fast_locals[var_idx].clone(), false, name.clone())
                    } else {
                        let fv_idx = idx - code.cellvars.len();
                        let name = code
                            .freevars
                            .get(fv_idx)
                            .ok_or_else(|| PyError::runtime_error("freevar index out of range"))?;
                        (f.closure.get(fv_idx).cloned(), true, name.clone())
                    }
                };
                if let Some(cell) = cell_ref {
                    let val = {
                        let obj = cell.borrow();
                        match &*obj {
                            PyObject::Cell { value: Some(inner) } => inner.clone(),
                            PyObject::Cell { value: None } => {
                                return Err(PyError::name_error_for(&name_str));
                            }
                            _ => cell.clone(),
                        }
                    };
                    self.frames[fi].push(val);
                } else if is_freevar {
                    let val = {
                        let globals = self.frames[fi].globals.borrow();
                        globals.get(&interner::intern(&name_str)).cloned()
                    };
                    if let Some(v) = val {
                        self.frames[fi].push(v);
                    } else {
                        let val = self.frames[fi]
                            .builtins
                            .get(&interner::intern(&name_str))
                            .cloned();
                        if let Some(v) = val {
                            self.frames[fi].push(v);
                        } else {
                            return Err(PyError::name_error_for(&name_str));
                        }
                    }
                } else {
                    return Err(PyError::name_error_for(&name_str));
                }
            }

            Opcode::STORE_DEREF => {
                let idx = arg as usize;
                let val = self.frames[fi].pop()?;
                let has_cellvars = idx < self.frames[fi].code.cellvars.len();
                if has_cellvars {
                    let name = &self.frames[fi].code.cellvars[idx];
                    let var_idx = self.frames[fi]
                        .code
                        .varnames
                        .iter()
                        .position(|&n| crate::interner::intern_eq(n, name))
                        .ok_or_else(|| PyError::runtime_error("variable not found"))?;
                    if var_idx < self.frames[fi].fast_locals.len() {
                        let existing_is_cell = matches!(&self.frames[fi].fast_locals[var_idx], Some(c) if matches!(&*c.borrow(), PyObject::Cell { .. }));
                        if existing_is_cell {
                            let cell = self.frames[fi].fast_locals[var_idx].clone().unwrap();
                            let mut cell_val = cell.borrow_mut();
                            if let PyObject::Cell { value } = &mut *cell_val {
                                *value = Some(val);
                            }
                        } else {
                            let new_cell = PyObjectRef::new(PyObject::Cell { value: Some(val) });
                            self.frames[fi].fast_locals[var_idx] = Some(new_cell);
                        }
                    } else {
                        let new_cell = PyObjectRef::new(PyObject::Cell { value: Some(val) });
                        self.frames[fi].fast_locals.push(Some(new_cell));
                    }
                } else {
                    let fv_idx = idx - self.frames[fi].code.cellvars.len();
                    let existing_is_cell = matches!(self.frames[fi].closure.get(fv_idx), Some(c) if matches!(&*c.borrow(), PyObject::Cell { .. }));
                    if existing_is_cell {
                        let cell = self.frames[fi].closure[fv_idx].clone();
                        let mut cell_val = cell.borrow_mut();
                        if let PyObject::Cell { value } = &mut *cell_val {
                            *value = Some(val);
                        }
                    } else {
                        return Err(PyError::name_error(format!(
                            "variable '{}' not found",
                            self.frames[fi]
                                .code
                                .freevars
                                .get(fv_idx)
                                .map(|s| s.as_str())
                                .unwrap_or("?")
                        )));
                    }
                }
            }

            Opcode::DELETE_FAST => {
                let var_idx = arg as usize;
                let name = self.frames[fi].code.varnames[var_idx].to_string();
                self.frames[fi].remove_local(&name);
                if self.frames[fi].frame_locals_obj.is_some() {
                    self.sync_frame_locals(fi);
                }
            }

            Opcode::DELETE_NAME => {
                let name_idx = arg as usize;
                let name = self.frames[fi].code.names[name_idx].to_string();
                // Live exec-locals mapping: delete from it (NameError when
                // absent, even if frame globals has the name — CPython
                // deletes from f_locals).
                if let Some(deleted) =
                    crate::vm::helpers::frame_exec_local_del(&self.frames[fi], &name)
                {
                    if !deleted {
                        return Err(PyError::name_error_for(crate::interner::lookup_str(
                            self.frames[fi].code.names[name_idx],
                        )));
                    }
                    if self.frames[fi].frame_locals_obj.is_some() {
                        self.sync_frame_locals(fi);
                    }
                    return Ok(true);
                }
                if let Some(live_module) = self.frames[fi].live_module.clone() {
                    if let PyObject::Module { dict, .. } = &mut *live_module.borrow_mut() {
                        dict.remove(&interner::intern(&name));
                    }
                }
                self.frames[fi]
                    .globals
                    .borrow_mut()
                    .remove(&interner::intern(&name));
                if let Some(prep) = self.frames[fi].prepared_namespace.clone() {
                    self.class_ns_del(&prep, &name)?;
                }
                if self.frames[fi].frame_locals_obj.is_some() {
                    self.sync_frame_locals(fi);
                }
            }

            _ => return Ok(false),
        }
        Ok(true)
    }
}
