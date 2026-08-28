use crate::bytecode::Opcode;
use crate::object::*;
use crate::vm::VirtualMachine;

impl VirtualMachine {
    pub(crate) fn handle_unpack(&mut self, fi: usize, op: Opcode, arg: u32) -> PyResult<bool> {
        match op {
            Opcode::UNPACK_SEQUENCE => {
                let count = arg as usize;
                let seq = self.frames[fi].pop()?;
                let list_items = {
                    let obj = seq.borrow();
                    match &*obj {
                        PyObject::List(v) | PyObject::Tuple(v) => Some(v.clone()),
                        _ => None,
                    }
                };
                let items = match list_items {
                    Some(v) => v,
                    None => {
                        let iterator =
                            crate::object::builtin_iter(&[seq.clone()]).map_err(|_| {
                                PyError::type_error(format!(
                                    "cannot unpack non-iterable '{}' object",
                                    seq.borrow().type_name()
                                ))
                            })?;
                        let mut v = Vec::new();
                        loop {
                            match crate::object::builtin_next(&[iterator.clone()]) {
                                Ok(val) => v.push(val),
                                Err(e) if crate::object::is_stop_iteration_error(&e) => break,
                                Err(e) => return Err(e),
                            }
                        }
                        v
                    }
                };
                if items.len() != count {
                    return Err(PyError::value_error(if items.len() < count {
                        format!(
                            "not enough values to unpack (expected {}, got {})",
                            count,
                            items.len()
                        )
                    } else {
                        format!("too many values to unpack (expected {})", count)
                    }));
                }
                for item in items.into_iter().rev() {
                    self.frames[fi].push(item);
                }
            }

            Opcode::UNPACK_EX => {
                let before = (arg >> 8) as usize;
                let after = (arg & 0xFF) as usize;
                let total = before + after + 1;
                let seq = self.frames[fi].pop()?;
                let list_items = {
                    let obj = seq.borrow();
                    match &*obj {
                        PyObject::List(v) | PyObject::Tuple(v) => Some(v.clone()),
                        _ => None,
                    }
                };
                let items = match list_items {
                    Some(v) => v,
                    None => {
                        let iterator =
                            crate::object::builtin_iter(&[seq.clone()]).map_err(|_| {
                                PyError::type_error(format!(
                                    "cannot unpack non-iterable '{}' object",
                                    seq.borrow().type_name()
                                ))
                            })?;
                        let mut v = Vec::new();
                        loop {
                            match crate::object::builtin_next(&[iterator.clone()]) {
                                Ok(val) => v.push(val),
                                Err(e) if crate::object::is_stop_iteration_error(&e) => break,
                                Err(e) => return Err(e),
                            }
                        }
                        v
                    }
                };
                if items.len() < before + after {
                    return Err(PyError::value_error(format!(
                        "not enough values to unpack (expected at least {}, got {})",
                        before + after,
                        items.len()
                    )));
                }
                let _ = total;
                let n = items.len();
                for i in (n - after)..n {
                    self.frames[fi].push(items[i].clone());
                }
                let star_count = n - before - after;
                let mut star_items: Vec<PyObjectRef> = Vec::new();
                for i in before..(before + star_count) {
                    star_items.push(items[i].clone());
                }
                self.frames[fi].push(py_list(star_items));
                for i in (0..before).rev() {
                    self.frames[fi].push(items[i].clone());
                }
            }

            _ => return Ok(false),
        }
        Ok(true)
    }
}
