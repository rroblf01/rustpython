use crate::bytecode::Opcode;
use crate::object::*;
use crate::vm::VirtualMachine;

impl VirtualMachine {
    pub(crate) fn handle_collections(&mut self, fi: usize, op: Opcode, arg: u32) -> PyResult<bool> {
        match op {
            Opcode::LIST_APPEND => {
                let val = self.frames[fi].pop()?;
                let list = self.frames[fi].peek(arg as usize)?;
                let mut obj = list.borrow_mut();
                if let PyObject::List(v) = &mut *obj {
                    v.push(val);
                } else {
                    return Err(PyError::runtime_error("LIST_APPEND on non-list"));
                }
            }

            Opcode::LIST_EXTEND => {
                let val = self.frames[fi].pop()?;
                let items: Vec<PyObjectRef> = {
                    let val_ref = val.borrow();
                    match &*val_ref {
                        PyObject::List(v) => v.clone(),
                        PyObject::Tuple(v) => v.clone(),
                        _ => {
                            drop(val_ref);
                            let iterator =
                                crate::object::builtin_iter(&[val.clone()]).map_err(|e| {
                                    PyError::type_error(e.to_string())
                                })?;
                            let mut result = Vec::new();
                            loop {
                                match crate::object::builtin_next(&[iterator.clone()]) {
                                    Ok(item) => result.push(item),
                                    Err(e) if crate::object::is_stop_iteration_error(&e) => break,
                                    Err(e) => return Err(e),
                                }
                            }
                            result
                        }
                    }
                };
                let list = self.frames[fi].peek(arg as usize)?;
                let mut obj = list.borrow_mut();
                if let PyObject::List(v) = &mut *obj {
                    v.extend(items);
                } else {
                    return Err(PyError::runtime_error("LIST_EXTEND on non-list"));
                }
            }

            Opcode::SET_ADD => {
                let val = self.frames[fi].pop()?;
                let set = self.frames[fi].peek(arg as usize)?;
                let mut obj = set.borrow_mut();
                if let PyObject::Set(v) = &mut *obj {
                    v.add(val)?;
                } else {
                    return Err(PyError::runtime_error("SET_ADD on non-set"));
                }
            }

            Opcode::SET_UPDATE => {
                let val = self.frames[fi].pop()?;
                let mut items = Vec::new();
                let it = crate::object::builtin_iter(&[val])?;
                loop {
                    match crate::object::builtin_next(&[it.clone()]) {
                        Ok(v) => items.push(v),
                        Err(e) if crate::object::is_stop_iteration_error(&e) => break,
                        Err(e) => return Err(e),
                    }
                }
                let set = self.frames[fi].peek(arg as usize)?;
                let mut obj = set.borrow_mut();
                if let PyObject::Set(v) = &mut *obj {
                    for item in items {
                        v.add(item)?;
                    }
                } else {
                    return Err(PyError::runtime_error("SET_UPDATE on non-set"));
                }
            }

            Opcode::MAP_ADD => {
                let val = self.frames[fi].pop()?;
                let key = self.frames[fi].pop()?;
                let map = self.frames[fi].peek(arg as usize)?;
                let mut obj = map.borrow_mut();
                if let PyObject::Dict(d) = &mut *obj {
                    d.set(key, val)?;
                } else {
                    return Err(PyError::runtime_error("MAP_ADD on non-dict"));
                }
            }

            Opcode::DICT_MERGE => {
                let source = self.frames[fi].pop()?;
                let target = self.frames[fi].peek(arg as usize)?;
                let source_items = {
                    let src_borrowed = source.borrow();
                    match &*src_borrowed {
                        PyObject::Dict(d) => d.items(),
                        _ => {
                            let mut out: Vec<(PyObjectRef, PyObjectRef)> = Vec::new();
                            let keys_obj =
                                crate::object::call_method_rebound(self, &source, "keys", vec![])?;
                            if let PyObject::List(items) = &*keys_obj.borrow() {
                                let keys: Vec<PyObjectRef> = items.clone();
                                for k in keys {
                                    let v = crate::object::call_method_rebound(
                                        self,
                                        &source,
                                        "__getitem__",
                                        vec![k.clone()],
                                    )?;
                                    out.push((k, v));
                                }
                            }
                            out
                        }
                    }
                };
                let mut target_borrowed = target.borrow_mut();
                if let PyObject::Dict(td) = &mut *target_borrowed {
                    for (k, v) in source_items {
                        td.set(k, v)?;
                    }
                } else {
                    return Err(PyError::runtime_error("DICT_MERGE on non-dict"));
                }
            }

            Opcode::LIST_TO_TUPLE => {
                let list = self.frames[fi].pop()?;
                let items = match &*list.borrow() {
                    PyObject::List(v) => v.clone(),
                    _ => return Err(PyError::runtime_error("LIST_TO_TUPLE on non-list")),
                };
                self.frames[fi].push(PyObjectRef::imm(PyObject::Tuple(items)));
            }

            _ => return Ok(false),
        }
        Ok(true)
    }
}
