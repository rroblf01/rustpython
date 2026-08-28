use crate::bytecode::Opcode;
use crate::interner::{self};
use crate::object::*;
use crate::vm::VirtualMachine;

impl VirtualMachine {
    pub(crate) fn handle_var(&mut self, fi: usize, op: Opcode, arg: u32) -> PyResult<bool> {
        match op {
            Opcode::NOP => { return Ok(true); }

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
                        .or_else(|| f.globals.borrow().get(&interner::intern(name)).cloned())
                        .or_else(|| {
                            f.module_globals
                                .as_ref()
                                .and_then(|mg| mg.borrow().get(&interner::intern(name)).cloned())
                        })
                        .or_else(|| f.builtins.get(&interner::intern(name)).cloned())
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
                self.frames[fi].globals.borrow_mut().insert(sid, val.clone());
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
                let name = crate::interner::lookup_str(frame.code.varnames[var_idx]);
                frame.insert_local(name, val);
                if frame.frame_locals_obj.is_some() {
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
                    let name = crate::interner::lookup_str(self.frames[fi].code.names[name_idx]);
                    let mut val = None;
                    {
                        let f = &self.frames[self.frames.len() - 1];
                        if let Some(v) = f.globals.borrow().get(&interner::intern(name)).cloned() {
                            val = Some(v);
                        } else if let Some(v) = f
                            .module_globals
                            .as_ref()
                            .and_then(|mg| mg.borrow().get(&interner::intern(name)).cloned())
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
                                    if let Some(v) = dict.get(&interner::intern(name)) {
                                        val = Some(v.clone());
                                    }
                                }
                            }
                            if val.is_none() {
                                if let Some(v) = f.builtins.get(&interner::intern(name)).cloned() {
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
                        None => return Err(PyError::name_error_for(name)),
                    }
                }
            }

            Opcode::STORE_GLOBAL => {
                let name_idx = arg as usize;
                let name = crate::interner::lookup(self.frames[fi].code.names[name_idx]);
                let val = self.frames[fi].pop()?;
                let sid = interner::intern(&name);
                self.frames[fi].globals.borrow_mut().insert(sid, val.clone());
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
                if let Some(live_module) = self.frames[fi].live_module.clone() {
                    if let PyObject::Module { dict, .. } = &mut *live_module.borrow_mut() {
                        dict.remove(&interner::intern(&name));
                    }
                }
                self.frames[fi]
                    .globals
                    .borrow_mut()
                    .remove(&interner::intern(&name));
                if self.frames[fi].frame_locals_obj.is_some() {
                    self.sync_frame_locals(fi);
                }
            }

            _ => return Ok(false),
        }
        Ok(true)
    }
}
