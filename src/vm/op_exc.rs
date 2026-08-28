use crate::bytecode::Opcode;
use crate::object::*;
use crate::vm::VirtualMachine;

impl VirtualMachine {
    pub(crate) fn handle_exc(&mut self, fi: usize, op: Opcode, arg: u32) -> PyResult<bool> {
        match op {
            Opcode::CHECK_EXC_MATCH => {
                let expected = self.frames[fi].pop()?;
                let exc = self.frames[fi].pop()?;
                let is_instance = matches!(&*exc.borrow(), PyObject::Instance { .. });
                let matched = if is_instance {
                    crate::object::builtin_isinstance(&[exc.clone(), expected.clone()])?.truthy()
                } else {
                    let typ_name = match &*exc.borrow() {
                        PyObject::Exception { typ, .. } => Some(typ.clone()),
                        PyObject::ExceptionGroup { typ, .. } => Some(typ.clone()),
                        _ => None,
                    };
                    match typ_name {
                        Some(t) => crate::vm::util::exc_type_matches(&expected, &t)?,
                        None => false,
                    }
                };
                self.frames[fi].push(py_bool(matched));
            }

            Opcode::CHECK_EXC_MATCH_STAR => {
                let expected = self.frames[fi].pop()?;
                let exc_dup = self.frames[fi].pop()?;
                let exc_orig = self.frames[fi].pop()?;
                let is_eg = match &*exc_dup.borrow() {
                    PyObject::ExceptionGroup { .. } => true,
                    _ => false,
                };
                if is_eg {
                    let (typ, args, matched, unmatched) = {
                        let eg = &*exc_dup.borrow();
                        let (typ, args, exceptions) = match eg {
                            PyObject::ExceptionGroup {
                                typ,
                                args,
                                exceptions,
                            } => (typ.clone(), args.clone(), exceptions.clone()),
                            _ => unreachable!(),
                        };
                        let mut matched = Vec::new();
                        let mut unmatched = Vec::new();
                        for child in &exceptions {
                            let child_name = match &*child.borrow() {
                                PyObject::Exception { typ, .. } => typ.clone(),
                                PyObject::ExceptionGroup { typ, .. } => typ.clone(),
                                _ => String::new(),
                            };
                            if crate::vm::util::exc_type_matches(&expected, &child_name)? {
                                matched.push(child.clone());
                            } else {
                                unmatched.push(child.clone());
                            }
                        }
                        (typ, args, matched, unmatched)
                    };
                    if !matched.is_empty() {
                        let matched_group = PyObjectRef::new(PyObject::ExceptionGroup {
                            typ: typ.clone(),
                            args: args.clone(),
                            exceptions: matched,
                        });
                        if !unmatched.is_empty() {
                            let unmatched_group = PyObjectRef::new(PyObject::ExceptionGroup {
                                typ: typ.clone(),
                                args: vec![py_str(&typ)],
                                exceptions: unmatched,
                            });
                            self.frames[fi].push(unmatched_group);
                        } else {
                            let empty_group = PyObjectRef::new(PyObject::ExceptionGroup {
                                typ: typ.clone(),
                                args: vec![py_str(&typ)],
                                exceptions: vec![],
                            });
                            self.frames[fi].push(empty_group);
                        }
                        self.frames[fi].push(matched_group);
                        self.frames[fi].push(py_bool(true));
                    } else {
                        self.frames[fi].push(exc_orig);
                        self.frames[fi].push(py_bool(false));
                    }
                } else {
                    let typ_name = match &*exc_dup.borrow() {
                        PyObject::Exception { typ, .. } => Some(typ.clone()),
                        _ => None,
                    };
                    let matched = match typ_name {
                        Some(t) => crate::vm::util::exc_type_matches(&expected, &t)?,
                        None => false,
                    };
                    if matched {
                        let empty_group = PyObjectRef::new(PyObject::ExceptionGroup {
                            typ: "ExceptionGroup".to_string(),
                            args: vec![py_str("")],
                            exceptions: vec![],
                        });
                        self.frames[fi].push(empty_group);
                        self.frames[fi].push(exc_dup);
                        self.frames[fi].push(py_bool(true));
                    } else {
                        self.frames[fi].push(exc_orig);
                        self.frames[fi].push(py_bool(false));
                    }
                }
            }

            Opcode::RERAISE => {
                let reraise_exc = if let Some(exc) = self.frames[fi].active_exception.take() {
                    *exc
                } else if let Ok(exc) = self.frames[fi].pop() {
                    exc
                } else if let Some((exc, _)) = self.exc_context_stack.last().cloned() {
                    exc
                } else {
                    if std::env::var("RPY_DEBUG_RERAISE").is_ok() {
                        eprintln!(
                            "RERAISE FAIL: func={} file={} stack_len={}",
                            self.frames[fi].code.name,
                            self.frames[fi].code.filename,
                            self.frames[fi].stack.len()
                        );
                    }
                    return Err(PyError::runtime_error("No active exception to re-raise"));
                };
                let is_empty_eg = match &*reraise_exc.borrow() {
                    PyObject::ExceptionGroup { exceptions, .. } => exceptions.is_empty(),
                    _ => false,
                };
                if !is_empty_eg {
                    if std::env::var("RPY_DEBUG_RERAISE").is_ok() {
                        eprintln!(
                            "RERAISE: kind={:?} repr={}",
                            std::mem::discriminant(&*reraise_exc.borrow()),
                            reraise_exc.borrow().repr()
                        );
                    }
                    return Err(PyError::Exception("re-raise".to_string(), reraise_exc));
                }
            }

            Opcode::RAISE_VARARGS => {
                let nargs = arg;
                match nargs {
                    0 => {
                        let reraise_exc = if let Some(exc) = self.frames[fi].active_exception.take()
                        {
                            Some(*exc)
                        } else if let Some(exc) = self.frames[fi].stack.pop() {
                            Some(exc)
                        } else {
                            self.exc_context_stack.last().map(|(e, _)| e.clone())
                        };
                        match reraise_exc {
                            Some(exc) => {
                                return Err(PyError::Exception(format!("re-raise"), exc));
                            }
                            None => {
                                return Err(PyError::runtime_error(
                                    "No active exception to re-raise",
                                ))
                            }
                        }
                    }
                    1 => {
                        let exc = self.frames[fi].pop()?;
                        let is_callable = !matches!(
                            &*exc.borrow(),
                            PyObject::Exception { .. }
                                | PyObject::ExceptionGroup { .. }
                                | PyObject::Instance { .. }
                        );
                        let exc = if is_callable {
                            let exc_clone = exc.clone();
                            match self.call_function(exc_clone, vec![], vec![]) {
                                Ok(instance) => instance,
                                Err(e) => return Err(e),
                            }
                        } else {
                            exc
                        };
                        if let PyObject::Instance { typ, .. } = &*exc.borrow() {
                            if crate::object::find_exception_base_name(typ).is_none() {
                                return Err(PyError::type_error(
                                    "exceptions must derive from BaseException",
                                ));
                            }
                        }
                        let msg = match &*exc.borrow() {
                            PyObject::Str(s) => s.to_string(),
                            PyObject::Exception { args, .. } => {
                                if !args.is_empty() {
                                    args[0].str()
                                } else {
                                    "".to_string()
                                }
                            }
                            PyObject::ExceptionGroup { args, .. } => {
                                if !args.is_empty() {
                                    args[0].str()
                                } else {
                                    "".to_string()
                                }
                            }
                            PyObject::Instance { dict, .. } => {
                                let args = dict.get_str("args");
                                if let Some(a) = args {
                                    let b = a.borrow();
                                    if let PyObject::Tuple(t) = &*b {
                                        if !t.is_empty() {
                                            t[0].str()
                                        } else {
                                            exc.repr()
                                        }
                                    } else {
                                        exc.repr()
                                    }
                                } else {
                                    exc.repr()
                                }
                            }
                            _ => {
                                return Err(PyError::type_error(
                                    "exceptions must be str or Exception instances",
                                ))
                            }
                        };
                        if msg.is_empty() {
                            let exc_borrowed = exc.borrow();
                            let is_stop = match &*exc_borrowed {
                                PyObject::Exception { ref typ, .. } if typ == "StopIteration" => true,
                                PyObject::Type { name, .. } if name == "StopIteration" => true,
                                _ => false,
                            };
                            if is_stop {
                                return Err(PyError::StopIteration);
                            }
                        }
                        self.exc_type = Some(self.exception_class_of(&exc));
                        self.exc_value = Some(exc.clone());
                        self.exc_traceback = Some(py_none());
                        if std::env::var("RPY_DEBUG_EXCINFO").is_ok() {
                            eprintln!(
                                "RAISE set exc_type={} exc_value={}",
                                self.exc_type.as_ref().unwrap().repr(),
                                self.exc_value.as_ref().unwrap().repr()
                            );
                        }
                        self.capture_exception_context(&exc);
                        return Err(PyError::Exception(msg, exc));
                    }
                    2 => {
                        let cause = self.frames[fi].pop()?;
                        let exc = self.frames[fi].pop()?;
                        let exc = {
                            let is_callable = !matches!(
                                &*exc.borrow(),
                                PyObject::Exception { .. }
                                    | PyObject::ExceptionGroup { .. }
                                    | PyObject::Instance { .. }
                            );
                            if is_callable {
                                let exc_clone = exc.clone();
                                match self.call_function(exc_clone, vec![], vec![]) {
                                    Ok(instance) => instance,
                                    Err(e) => return Err(e),
                                }
                            } else {
                                exc
                            }
                        };
                        let cause_is_none = matches!(&*cause.borrow(), PyObject::None);
                        if !cause_is_none {
                            let cause_kind = match &*cause.borrow() {
                                PyObject::Exception { .. } | PyObject::ExceptionGroup { .. } => "exc",
                                PyObject::Instance { typ, .. } => {
                                    if crate::object::find_exception_base_name(typ).is_some() {
                                        "exc"
                                    } else {
                                        "bad"
                                    }
                                }
                                PyObject::Type { .. } => "class",
                                PyObject::BuiltinFunction { name, .. } => {
                                    if crate::vm::is_exception_subclass(name, "BaseException") {
                                        "class"
                                    } else {
                                        "bad"
                                    }
                                }
                                _ => "bad",
                            };
                            if cause_kind == "bad" {
                                return Err(PyError::type_error(
                                    "exception causes must derive from BaseException",
                                ));
                            }
                        }
                        let cause = if cause_is_none {
                            cause
                        } else {
                            let is_callable = !matches!(
                                &*cause.borrow(),
                                PyObject::Exception { .. }
                                    | PyObject::ExceptionGroup { .. }
                                    | PyObject::Instance { .. }
                            );
                            if is_callable {
                                let cause_clone = cause.clone();
                                match self.call_function(cause_clone, vec![], vec![]) {
                                    Ok(instance) => instance,
                                    Err(e) => return Err(e),
                                }
                            } else {
                                cause
                            }
                        };
                        if !cause_is_none {
                            let is_exc = match &*cause.borrow() {
                                PyObject::Exception { .. } | PyObject::ExceptionGroup { .. } => true,
                                PyObject::Instance { typ, .. } => {
                                    crate::object::find_exception_base_name(typ).is_some()
                                }
                                _ => false,
                            };
                            if !is_exc {
                                return Err(PyError::type_error(
                                    "exception causes must derive from BaseException",
                                ));
                            }
                        }
                        let exc_msg = match &*exc.borrow() {
                            PyObject::Exception { args, .. } => {
                                if !args.is_empty() {
                                    args[0].str()
                                } else {
                                    exc.str()
                                }
                            }
                            _ => exc.str(),
                        };
                        let cause_msg = match &*cause.borrow() {
                            PyObject::Exception { args, .. } => {
                                if !args.is_empty() {
                                    args[0].str()
                                } else {
                                    cause.str()
                                }
                            }
                            _ if cause_is_none => String::new(),
                            _ => cause.str(),
                        };
                        match &mut *exc.borrow_mut() {
                            PyObject::Exception {
                                cause: ref mut cause_field,
                                suppress_context: ref mut suppress_field,
                                ..
                            } => {
                                let is_none = matches!(&*cause.borrow(), PyObject::None);
                                *cause_field = if cause_is_none {
                                    None
                                } else {
                                    Some(cause.clone())
                                };
                                *suppress_field = cause_is_none;
                            }
                            PyObject::Instance { dict, .. } => {
                                dict.insert_str("__cause__", cause.clone());
                                dict.insert_str("__suppress_context__", py_bool(cause_is_none));
                            }
                            _ => {}
                        }
                        let err_msg = if cause_msg.is_empty() {
                            exc_msg
                        } else {
                            format!("{} (caused by: {})", exc_msg, cause_msg)
                        };
                        self.capture_exception_context(&exc);
                        return Err(PyError::Exception(err_msg, exc));
                    }
                    _ => return Err(PyError::runtime_error("invalid RAISE_VARARGS count")),
                }
            }

            _ => return Ok(false),
        }
        Ok(true)
    }
}
