use crate::interner::{self, StrId};
use crate::object::*;
use crate::vm::VirtualMachine;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

impl VirtualMachine {
    pub fn execute(&mut self) -> PyResult<PyObjectRef> {
        // ALWAYS install this machine and RESTORE the previous one on exit:
        // leaving a pooled/disposable machine's pointer installed after it
        // was recycled made later with_vm_mut calls read garbage fields
        // (observed: recursion_limit=16, frames.len()=garbage).
        let prev_vm_ptr =
            crate::object::VM_PTR.with(|p| p.replace(Some(self as *mut VirtualMachine)));

        // Every call site that pushes a frame onto `self.frames` immediately
        // calls `execute()` and pops exactly that one frame once it returns
        // (see exec_code, call_function's Function arm, __build_class__,
        // generator/coroutine drivers) — so for the entire lifetime of this
        // `execute_inner` invocation, `self.frames[frame_floor]` is *this*
        // call's own frame, and any frames below it belong to an outer,
        // currently-suspended `execute()` call further down the Rust stack.
        // Bounding exception handling to `frame_floor` matters: without it,
        // an uncaught exception from a nested call (a Python function call,
        // a module body during import, ...) would find and "handle" itself
        // using an outer/caller frame's try/except — while that outer frame
        // was not actually the one executing, and the intervening frame(s)
        // were never popped/unwound. Instead, nested calls must propagate an
        // unhandled exception as a plain Err all the way back to their own
        // call site (which pops its own frame), letting the *caller's own*
        // execute_inner loop (now correctly with its own frame on top) find
        // the enclosing handler itself.
        let frame_floor = self.frames.len() - 1;
        let result = self.execute_inner(frame_floor);
        // Store exception info for sys.exc_info()
        if let Err(ref e) = result {
            // Must be the real exception object + its real class (what
            // `sys.exc_info()` returns), not a bare descriptive string —
            // this is what let `issubclass(sys.exc_info()[0], ...)` crash
            // with "arg 1 must be a class" for ANY natively-raised error
            // (a `TypeError`/`ValueError`/etc. raised internally by a
            // builtin/opcode rather than a Python-level `raise` statement,
            // which instead goes through RAISE_VARARGS's own now-fixed
            // assignment) — exactly the same bug, just a second, separate
            // site that produced it for a different class of raise.
            let exc_obj = Self::error_to_exc_obj(e);
            self.exc_type = Some(self.exception_class_of(&exc_obj));
            self.exc_value = Some(exc_obj);
        }
        // Restore the caller's machine and exception context (see saves at
        // the top of this function).
        crate::object::VM_PTR.with(|p| p.set(prev_vm_ptr));
        result
    }

    /// Injects `err` at the current suspension point of the single frame
    /// already pushed onto `self.frames` (used by generator/coroutine
    /// `.throw()`), then resumes normal execution. Mirrors `execute()`'s
    /// frame_floor bookkeeping but starts by searching for a handler for
    /// `err` instead of running the next instruction — this is what lets a
    /// `try/finally` wrapping the suspended `yield` actually see the thrown
    /// exception and run its cleanup, exactly as CPython's generator throw
    /// does. Returns Err(err) unchanged if the generator's own code has no
    /// handler for it (caller propagates it to whoever called .throw()).
    pub fn import_cached_or_fresh(&mut self, name: &str) -> Option<PyObjectRef> {
        let prioritize_sys = crate::vm::SYS_MODULES_PRIORITY.with(|c| c.get());
        if prioritize_sys {
            if let Some(sys_mod) = self.modules.get("sys") {
                if let PyObject::Module { dict, .. } = &*sys_mod.borrow() {
                    if let Some(mod_dict) = dict.get_str("modules") {
                        let md = mod_dict.borrow();
                        if let PyObject::Dict(d) = &*md {
                            if let Some(real) =
                                d.get(&crate::object::py_str(name)).ok().flatten()
                            {
                                return Some(real);
                            }
                        }
                    }
                }
            }
        }

        let module = self.modules.get(name)?.clone();
        let in_sys_modules = if let Some(sys_mod) = self.modules.get("sys") {
            if let PyObject::Module { dict, .. } = &*sys_mod.borrow() {
                if let Some(mod_dict) = dict.get_str("modules") {
                    let md = mod_dict.borrow();
                    if let PyObject::Dict(d) = &*md {
                        d.get(&crate::object::py_str(name)).ok().flatten().is_some()
                    } else {
                        false
                    }
                } else {
                    false
                }
            } else {
                false
            }
        } else {
            false
        };
        if in_sys_modules {
            return Some(module);
        }
        let fresh = PyObjectRef::new(PyObject::Module {
            name: name.to_string(),
            dict: {
                let b = module.borrow();
                if let PyObject::Module { dict, .. } = &*b {
                    dict.clone()
                } else {
                    Box::new(crate::object::TypeDict::default())
                }
            },
        });
        self.modules.insert(name.to_string(), fresh.clone());
        if let Some(sys_mod) = self.modules.get("sys") {
            if let PyObject::Module { dict, .. } = &*sys_mod.borrow() {
                if let Some(mod_dict) = dict.get_str("modules") {
                    if let PyObject::Dict(d) = &mut *mod_dict.borrow_mut() {
                        let _ = d.set(crate::object::py_str(name), fresh.clone());
                    }
                }
            }
        }
        Some(fresh)
    }

    pub(crate) fn throw_into_frame(&mut self, err: PyError) -> PyResult<PyObjectRef> {
        let frame_floor = self.frames.len() - 1;
        if !self.handle_exception(&err, frame_floor) {
            return Err(err);
        }
        let result = self.execute_inner(frame_floor);
        if let Err(ref e) = result {
            // Must be the real exception object + its real class (what
            // `sys.exc_info()` returns), not a bare descriptive string —
            // this is what let `issubclass(sys.exc_info()[0], ...)` crash
            // with "arg 1 must be a class" for ANY natively-raised error
            // (a `TypeError`/`ValueError`/etc. raised internally by a
            // builtin/opcode rather than a Python-level `raise` statement,
            // which instead goes through RAISE_VARARGS's own now-fixed
            // assignment) — exactly the same bug, just a second, separate
            // site that produced it for a different class of raise.
            let exc_obj = Self::error_to_exc_obj(e);
            self.exc_type = Some(self.exception_class_of(&exc_obj));
            self.exc_value = Some(exc_obj);
        }
        result
    }

    fn execute_inner(&mut self, frame_floor: usize) -> PyResult<PyObjectRef> {
        loop {
            let result = self.execute_instruction();
            match result {
                Ok(None) => continue,
                Ok(Some(val)) => return Ok(val),
                Err(e) => {
                    if matches!(&e, PyError::SystemExit(_)) {
                        return Err(e);
                    }
                    if std::env::var("RPY_DEBUG_EXC").is_ok() {
                        eprintln!(
                            "handle_exception: frame_floor={} frames.len()={} err={}",
                            frame_floor,
                            self.frames.len(),
                            e
                        );
                    }
                    if !self.handle_exception(&e, frame_floor) {
                        // This execute() call's own frame has no handler for `e` — it
                        // will propagate as a plain Err up to our Rust caller, which
                        // pops this frame. Record this frame's info before that
                        // happens; as the error keeps propagating outward, each
                        // enclosing execute() level prepends its own frame here too,
                        // building the traceback outermost-first (only cleared when
                        // some level below DOES catch it — see the `else` branch).
                        if let Some(f) = self.frames.get(frame_floor) {
                            let idx =
                                f.ip.saturating_sub(1)
                                    .min(f.code.instructions.len().saturating_sub(1));
                            let line = f.code.line_number(idx);
                            // Prepend a real `types.TracebackType` node for this
                            // frame onto the escaping exception's __traceback__.
                            // Clone what's needed first: `prepend_traceback` needs
                            // `&mut self`, which can't coexist with the `f` borrow.
                            let (code, globals, ip, filename, name) = (
                                f.code.clone(),
                                f.globals.clone(),
                                f.ip,
                                crate::interner::lookup_str(f.code.filename).to_string(),
                                crate::interner::lookup_str(f.code.name).to_string(),
                            );
                            let exc_obj = Self::error_to_exc_obj(&e);
                            let frame_idx = frame_floor;
                            drop(f);
                            self.prepend_traceback(&exc_obj, frame_idx);
                            // Each enclosing level re-runs this same branch as
                            // the error keeps propagating outward — only the
                            // FIRST (innermost, deepest) occurrence should set
                            // `last_error_line`/`file` (matching the ORIGINAL
                            // per-instruction-update behavior, which always
                            // reflected wherever execution last was, i.e. the
                            // innermost frame, before the error started
                            // unwinding). `last_traceback` is still empty only
                            // on this first, innermost pass.
                            if self.last_traceback.is_empty() {
                                self.last_error_line = Some(line);
                                self.last_error_file = Some(filename.clone());
                            }
                            self.last_traceback.insert(0, (filename, line, name));
                        }
                        return Err(e);
                    } else {
                        // Exception was actually caught somewhere — any traceback
                        // entries accumulated so far (from inner frames that didn't
                        // handle it) no longer describe a real escaping error.
                        self.last_traceback.clear();
                        // The catching frame's __traceback__ node was prepended
                        // inside `handle_exception` (on the SAME object the
                        // handler binds — done there, not here, because
                        // synthesized PyErrors produce a fresh object on every
                        // `error_to_exc_obj` call).
                    }
                    if std::env::var("RPY_DEBUG_EXC").is_ok() {
                        eprintln!(
                            "  handled: frames.len()={} top_stack_len={}",
                            self.frames.len(),
                            self.frames.last().map(|f| f.stack.len()).unwrap_or(0)
                        );
                    }
                }
            }
        }
    }
}
