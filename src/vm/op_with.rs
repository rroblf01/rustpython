use crate::bytecode::Opcode;
use crate::interner;
use crate::object::*;
use crate::vm::VirtualMachine;

impl VirtualMachine {
    pub(crate) fn handle_with(&mut self, fi: usize, op: Opcode, arg: u32) -> PyResult<bool> {
        let _ = arg;
        match op {
            Opcode::BEFORE_ASYNC_WITH => {
                let mgr = self.frames[fi].pop()?;
                // Check __aexit__ first (mirroring SETUP_WITH's __exit__ check)
                let aexit = mgr.borrow().get_attribute("__aexit__").ok();
                if aexit.is_none() {
                    let has_enter = mgr.borrow().get_attribute("__enter__").is_ok();
                    let has_exit = mgr.borrow().get_attribute("__exit__").is_ok();
                    let supports_sync = has_enter && has_exit;
                    return Err(PyError::type_error(if supports_sync {
                        "object does not support the asynchronous context manager protocol (missed __aexit__ method) but it supports the context manager protocol. Did you mean to use 'with'?"
                    } else {
                        "object does not support the asynchronous context manager protocol (missed __aexit__ method)"
                    }));
                }
                let aenter_raw = mgr.borrow().get_attribute("__aenter__").ok();
                if let Some(aenter_raw) = aenter_raw {
                    let is_builtin = matches!(&*aenter_raw.borrow(), PyObject::BuiltinMethod { .. });
                    let bound = if is_builtin {
                        let b = aenter_raw.borrow();
                        match &*b {
                            PyObject::BuiltinMethod { name, func, .. } => {
                                PyObjectRef::imm(PyObject::BuiltinMethod {
                                    name: name.clone(),
                                    func: *func,
                                    self_obj: mgr.clone(),
                                })
                            }
                            _ => unreachable!(),
                        }
                    } else {
                        PyObjectRef::imm(PyObject::BoundMethod {
                            func: aenter_raw,
                            self_obj: mgr.clone(),
                        })
                    };
                    let result = self.call_function(bound, vec![], vec![])?;
                    self.frames[fi].push(mgr);
                    self.frames[fi].push(result);
                } else {
                    return Err(PyError::type_error(
                        "object does not support the asynchronous context manager protocol (missed __aenter__ method)",
                    ));
                }
            }

            Opcode::SETUP_WITH => {
                // Look up __enter__ and call it, keeping manager on stack
                let mgr = self.frames[fi].peek(0)?;
                let exit_method = mgr.borrow().get_attribute("__exit__").ok();
                if exit_method.is_none() {
                    let has_aenter = mgr
                        .borrow()
                        .get_attribute("__aenter__")
                        .is_ok_and(|v| !matches!(&*v.borrow(), PyObject::None));
                    return Err(PyError::type_error(if has_aenter {
                        "object does not support the context manager protocol (missed __exit__ method) \
                         but it supports the asynchronous context manager protocol. \
                         Did you mean to use 'async with'?"
                    } else {
                        "object does not support the context manager protocol (missed __exit__ method)"
                    }));
                }
                let enter_raw = mgr.borrow().get_attribute("__enter__").ok();
                if let Some(enter_raw) = enter_raw {
                    let is_builtin = matches!(&*enter_raw.borrow(), PyObject::BuiltinMethod { .. });
                    let enter = if is_builtin {
                        let b = enter_raw.borrow();
                        match &*b {
                            PyObject::BuiltinMethod { name, func, .. } => {
                                PyObjectRef::imm(PyObject::BuiltinMethod {
                                    name: name.clone(),
                                    func: *func,
                                    self_obj: mgr.clone(),
                                })
                            }
                            _ => unreachable!(),
                        }
                    } else {
                        PyObjectRef::imm(PyObject::BoundMethod {
                            func: enter_raw,
                            self_obj: mgr.clone(),
                        })
                    };
                    let result = self.call_function(enter, vec![], vec![])?;
                    self.frames[fi].push(result);
                } else {
                    return Err(PyError::type_error(
                        "object does not support the context manager protocol (missed __enter__ method)",
                    ));
                }
            }

            Opcode::WITH_EXIT => {
                // Stack: [..., exception_obj, manager]
                // Call manager.__exit__(exc_type, exc_val, traceback) — exc_type
                // and exc_val must be the real class object and exception
                // instance (not a bare type-name string / the first ctor arg),
                // since __exit__ implementations commonly do isinstance(value,
                // ...), re-raise `value`, or read value.args/__traceback__.
                let mgr = self.frames[fi].pop()?;
                let (typ_obj, val) = {
                    let exc = self.frames[fi].peek(0)?;
                    let exc_borrowed = exc.borrow();
                    match &*exc_borrowed {
                        PyObject::Exception { typ, .. } => {
                            let typ_obj = self.frames[fi]
                                .builtins
                                .get(&interner::intern(&typ))
                                .cloned()
                                .unwrap_or_else(|| py_str(typ));
                            (typ_obj, exc.clone())
                        }
                        PyObject::Instance { typ, .. } => (typ.clone(), exc.clone()),
                        _ => (py_none(), py_none()),
                    }
                };
                let exit_raw = mgr
                    .borrow()
                    .get_attribute("__exit__")
                    .map_err(|_| PyError::attribute_error("context manager has no __exit__"))?;
                let is_builtin = matches!(&*exit_raw.borrow(), PyObject::BuiltinMethod { .. });
                let bound = if is_builtin {
                    let b = exit_raw.borrow();
                    match &*b {
                        PyObject::BuiltinMethod { name, func, .. } => {
                            PyObjectRef::imm(PyObject::BuiltinMethod {
                                name: name.clone(),
                                func: *func,
                                self_obj: mgr.clone(),
                            })
                        }
                        _ => unreachable!(),
                    }
                } else {
                    PyObjectRef::imm(PyObject::BoundMethod {
                        func: exit_raw,
                        self_obj: mgr,
                    })
                };
                let tb_arg = val
                    .borrow()
                    .get_attribute("__traceback__")
                    .unwrap_or_else(|_| py_none());
                let result = self.call_function(bound, vec![typ_obj, val, tb_arg], vec![])?;
                self.frames[fi].push(result);
            }

            _ => return Ok(false),
        }
        Ok(true)
    }
}
