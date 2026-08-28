use crate::bytecode::Opcode;
use crate::interner;
use crate::object::*;
use crate::vm::VirtualMachine;
use std::collections::HashMap;
use std::rc::Rc;

impl VirtualMachine {
    pub(crate) fn handle_call(&mut self, fi: usize, op: Opcode, arg: u32) -> PyResult<()> {
        match op {
            Opcode::PUSH_NULL => {
                self.frames[fi].push(py_none());
            }

            Opcode::CALL => {
                let npos = arg as usize & 0xFF;
                let nkw = (arg as usize >> 8) & 0xFF;
                let stack_len = self.frames[fi].stack.len();

                if nkw == 0 && npos + 1 <= stack_len {
                    let split = stack_len - 1 - npos;
                    let callable = self.frames[fi].stack.remove(split);
                    let args: Vec<PyObjectRef> =
                        self.frames[fi].stack.drain(split..).collect();

                    {
                        let is_special_throw = matches!(
                            &*callable.borrow(),
                            PyObject::BuiltinMethod { func, .. }
                                if std::ptr::fn_addr_eq(
                                    *func,
                                    crate::object::generator_throw_fallback as crate::object::BuiltinFunc,
                                )
                        );
                        if !is_special_throw {
                            if let PyObject::BuiltinMethod { name, func, self_obj } =
                                &*callable.borrow()
                            {
                                if name == "append" && args.len() == 1 {
                                    let so = self_obj.clone();
                                    let mut b = so.borrow_mut();
                                    if let PyObject::List(items) = &mut *b {
                                        items.push(args[0].clone());
                                        drop(b);
                                        self.frames[fi].push(py_none());
                                        return Ok(());
                                    }
                                }
                                let mut na = Vec::with_capacity(args.len() + 1);
                                na.push(self_obj.clone());
                                na.extend(args.iter().cloned());
                                drop(callable.borrow());
                                let r = func(&na)?;
                                self.frames[fi].push(r);
                                return Ok(());
                            }
                        }
                    }

                    let result = self.call_function(callable, args, vec![])?;
                    self.frames[fi].push(result);
                    return Ok(());
                }

                let total_to_pop = npos + 2 * nkw;
                let mut items = Vec::with_capacity(total_to_pop);
                for _ in 0..total_to_pop {
                    if self.frames[fi].stack.len() > 1 {
                        items.push(self.frames[fi].pop()?);
                    } else {
                        break;
                    }
                }
                let callable = self.frames[fi].pop()?;
                items.reverse();
                let mut args = Vec::new();
                let mut keywords = Vec::new();
                let mut i = 0;
                while i < npos && i < items.len() {
                    args.push(items[i].clone());
                    i += 1;
                }
                while i + 1 < items.len() {
                    if let PyObject::Str(name) = &*items[i].borrow() {
                        keywords.push((name.to_string(), items[i + 1].clone()));
                        i += 2;
                    } else {
                        break;
                    }
                }
                let result = self.call_function(callable, args, keywords)?;
                self.frames[fi].push(result);
            }
            Opcode::MAKE_CELL => {
                let idx = arg as usize;
                let frame = &mut self.frames[fi];
                if idx < frame.fast_locals.len() {
                    let val = frame.fast_locals[idx].take();
                    let cell = PyObjectRef::new(PyObject::Cell { value: val });
                    frame.fast_locals[idx] = Some(cell);
                }
            }

            Opcode::COPY_FREE_VARS => {
                let nfree = arg as usize;
                let mut cells = Vec::with_capacity(nfree);
                for _ in 0..nfree {
                    cells.push(self.frames[fi].pop()?);
                }
                self.frames[fi].push(PyObjectRef::imm(PyObject::Tuple(cells)));
            }

            Opcode::MAKE_FUNCTION => {
                let has_closure = (arg & 0x100) != 0;
                let n_defaults = (arg & 0xFF) as usize;
                let n_kwdefaults = ((arg >> 9) & 0xFF) as usize;
                let mut kwdefaults = Vec::new();
                for _ in 0..n_kwdefaults {
                    kwdefaults.push(self.frames[fi].pop()?);
                }
                kwdefaults.reverse();
                let mut defaults = Vec::new();
                for _ in 0..n_defaults {
                    defaults.push(self.frames[fi].pop()?);
                }
                defaults.reverse();
                defaults.extend(kwdefaults);
                let code_obj = self.frames[fi].pop()?;
                let code = match &*code_obj.borrow() {
                    PyObject::Code(c) => c.clone(),
                    _ => {
                        return Err(PyError::runtime_error(
                            "MAKE_FUNCTION: expected code object",
                        ))
                    }
                };
                let closure = if has_closure {
                    let closure_tuple = self.frames[fi].pop()?;
                    let items = closure_tuple.borrow();
                    if let PyObject::Tuple(items) = &*items {
                        items.clone()
                    } else {
                        Vec::new()
                    }
                } else {
                    Vec::new()
                };
                let globals = self.frames[fi]
                    .module_globals
                    .clone()
                    .unwrap_or_else(|| self.frames[fi].globals.clone());
                let code_obj = code.clone();
                let func = PyObjectRef::new(PyObject::Function(Box::new(PyFunction {
                    code: code_obj.clone(),
                    globals,
                    defaults,
                    closure,
                    dict: HashMap::new(),
                    jit_ptr: std::cell::Cell::new(0),
                    jit_consts: std::cell::RefCell::new(Vec::new()),
                })));
                if let PyObject::Function(ref mut inner_f) = &mut *func.borrow_mut() {
                    let dict = &mut inner_f.dict;
                    dict.insert_str("__code__", PyObjectRef::imm(PyObject::Code(code_obj)));
                }
                let module_name_opt: Option<String> = if let Some(ref mg) = self.frames[fi].module_globals {
                    let mg = mg.borrow();
                    mg.get(&interner::intern("__name__")).and_then(|v| {
                        if let PyObject::Str(s) = &*v.borrow() { Some(s.to_string()) } else { None }
                    })
                } else {
                    let g = self.frames[fi].globals.borrow();
                    g.get(&interner::intern("__name__")).and_then(|v| {
                        if let PyObject::Str(s) = &*v.borrow() { Some(s.to_string()) } else { None }
                    })
                };
                if let Some(s) = module_name_opt {
                    if let PyObject::Function(ref mut inner_f) = &mut *func.borrow_mut() {
                        let dict = &mut inner_f.dict;
                        dict.insert_str("__module__", py_str(&s));
                    }
                }
                self.frames[fi].push(func);
            }

            _ => {}
        }
        Ok(())
    }
}
