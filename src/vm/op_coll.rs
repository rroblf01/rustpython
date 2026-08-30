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
                let target = self.frames[fi].peek(arg as usize)?.clone();
                let source_items = collect_mapping_items(self, &source)?;
                // Unlike DICT_UPDATE (dict displays), a call's `**source`
                // keyword unpacking must raise on ANY duplicate key —
                // whether it collides with an already-present keyword (an
                // explicit `k=v` already in the dict, or a PRIOR `**other`
                // merge) or with a duplicate key from THIS SAME source's
                // own iteration (a duck-typed multi-valued mapping whose
                // `keys()` yields one name twice — real CPython raises for
                // that too). Checking each item against the target
                // in-order, right before inserting it, catches both: the
                // second occurrence of a same-source duplicate hits the
                // check because the first occurrence already landed in
                // `target`.
                let fname = self.frames[fi]
                    .peek(arg as usize + 2)
                    .ok()
                    .map(|c| callable_display_name(&c))
                    .unwrap_or_else(|| "<call>".to_string());
                {
                    let mut target_borrowed = target.borrow_mut();
                    if let PyObject::Dict(td) = &mut *target_borrowed {
                        for (k, v) in source_items {
                            if td.get(&k)?.is_some() {
                                return Err(PyError::type_error(format!(
                                    "{}() got multiple values for keyword argument '{}'",
                                    fname,
                                    k.str()
                                )));
                            }
                            td.set(k, v)?;
                        }
                    } else {
                        return Err(PyError::runtime_error("DICT_MERGE on non-dict"));
                    }
                }
            }

            // `{**a, **b}` dict DISPLAY unpacking — unlike DICT_MERGE
            // above, silently overrides on a duplicate key (real CPython:
            // `PyDict_Merge(..., override=1)`), matching `{**{'x':1},
            // **{'x':2}}` == `{'x':2}`.
            Opcode::DICT_UPDATE => {
                let source = self.frames[fi].pop()?;
                let target = self.frames[fi].peek(arg as usize)?;
                let source_items = collect_mapping_items(self, &source)?;
                let mut target_borrowed = target.borrow_mut();
                if let PyObject::Dict(td) = &mut *target_borrowed {
                    for (k, v) in source_items {
                        td.set(k, v)?;
                    }
                } else {
                    return Err(PyError::runtime_error("DICT_UPDATE on non-dict"));
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

/// Extract `(key, value)` pairs from a mapping being `**`-unpacked, for
/// `DICT_MERGE`/`DICT_UPDATE`. A real dict reads its items directly; any
/// other mapping goes through the `keys()`/`__getitem__` protocol (a
/// dict-like class, e.g. `OrderedDict` or a duck-typed multi-valued
/// mapping) — `keys()` returns a `dict_keys` VIEW object on a real
/// dict/subclass, not a plain list, so it's collected generically via
/// `collect_iterable` rather than assumed to already be a `PyObject::List`.
fn collect_mapping_items(
    vm: &mut VirtualMachine,
    source: &PyObjectRef,
) -> PyResult<Vec<(PyObjectRef, PyObjectRef)>> {
    let is_dict = matches!(&*source.borrow(), PyObject::Dict(_));
    if is_dict {
        if let PyObject::Dict(d) = &*source.borrow() {
            return Ok(d.items());
        }
    }
    let mut out: Vec<(PyObjectRef, PyObjectRef)> = Vec::new();
    let keys_obj = crate::object::call_method_rebound(vm, source, "keys", vec![])?;
    let keys = crate::object::collect_iterable(&keys_obj)?;
    for k in keys {
        let v = crate::object::call_method_rebound(vm, source, "__getitem__", vec![k.clone()])?;
        out.push((k, v));
    }
    Ok(out)
}

/// Best-effort callable name for a `DICT_MERGE` "got multiple values for
/// keyword argument" TypeError — matches the name real CPython's own
/// message uses (a plain function's `__name__`, a bound/native method's
/// name, or a generic fallback for anything else rather than failing the
/// whole error path over a cosmetic detail).
fn callable_display_name(callable: &PyObjectRef) -> String {
    match &*callable.borrow() {
        PyObject::Function(f) => crate::interner::lookup_str(f.code.name).to_string(),
        PyObject::BuiltinFunction { name, .. } | PyObject::BuiltinMethod { name, .. } => {
            name.clone()
        }
        PyObject::BoundMethod { func, .. } => callable_display_name(func),
        other => other.type_name().to_string(),
    }
}
