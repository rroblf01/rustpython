use crate::bytecode::*;
use crate::interner::{self, InternedMap, StrId};
use crate::object::*;
use crate::vm::VirtualMachine;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use num_bigint::BigInt;
use num_traits::ToPrimitive;
use smallvec::SmallVec;

impl VirtualMachine {
    pub(crate) fn synth_exception(typ: &str, error: &PyError) -> PyObjectRef {
        // MUST be `Mut` (via `PyObjectRef::new`), not `Imm` — this converts
        // EVERY native `PyError::TypeError`/`ValueError`/`ZeroDivisionError`/
        // etc. (i.e. almost every runtime error the interpreter itself
        // detects, as opposed to a user `raise SomeError(...)` statement,
        // which already goes through `exceptions_ctor.rs`'s correctly-`Mut`
        // constructor) into a Python-visible exception object. `STORE_ATTR`
        // unconditionally rejects setting ANY attribute on an `Imm`-wrapped
        // value (see its own doc comment) before ever reaching
        // `PyObject::Exception`'s own (already-correct, already-permissive)
        // `set_attribute` arm — so with the old `imm` constructor here,
        // `except TypeError as e: e.__traceback__ = tb` (an extremely
        // common idiom: `unittest`'s own `result.py`, `contextlib`'s
        // generator-context-manager `__exit__`, ...) raised `AttributeError`
        // for literally any exception synthesized this way. Confirmed via
        // CPython's own test suite: this exact bug surfaced across 24
        // DIFFERENT test files simultaneously (the single widest-reaching
        // bug found this whole session), all via this one shared root cause.
        PyObjectRef::new(PyObject::Exception {
            typ: typ.to_string(),
            args: vec![py_str(&error.message())],
            cause: None,
            suppress_context: false,
            context: None,
            traceback: None,
            extra: None,
        })
    }

    /// The real CLASS object behind a raised exception instance — what
    /// `sys.exc_info()[0]` must be (see the `RAISE_VARARGS` call site that
    /// uses this). For a `class MyError(Exception): ...` instance this is
    /// its own `typ`; for the native `PyObject::Exception`/`ExceptionGroup`
    /// representations (a bare string type name, not a real class object)
    /// this looks the name up in `self.builtins` (where every builtin
    /// exception is registered as a `BuiltinFunction`/constructor) — falling
    /// back to a freshly-built placeholder `Type` sharing just the name if
    /// somehow not found there, rather than ever returning the instance
    /// itself (which is what caused `issubclass(exc_info()[0], ...)` to
    /// raise "arg 1 must be a class").
    pub(crate) fn exception_class_of(&self, exc: &PyObjectRef) -> PyObjectRef {
        let name = match &*exc.borrow() {
            PyObject::Instance { typ, .. } => return typ.clone(),
            PyObject::Exception { typ, .. } => typ.clone(),
            PyObject::ExceptionGroup { .. } => "ExceptionGroup".to_string(),
            other => other.type_name().to_string(),
        };
        if let Some(builtin) = self.builtins.get(&interner::intern(&name)) {
            return builtin.clone();
        }
        PyObjectRef::new(PyObject::Type {
            name,
            dict: Box::new(TypeDict::default()),
            bases: vec![],
            mro: vec![],
        })
    }

    /// PEP 3134 implicit exception chaining: when a NEW exception is raised
    /// while another is being handled (an `except` handler is on
    /// `exc_context_stack`) or still propagating (finally-over-failure), the
    /// new exception's `__context__` points at it. Called from RAISE_VARARGS
    /// before the exception propagates; also records the new exception as the
    /// currently-propagating one.
    pub(crate) fn capture_exception_context(&mut self, exc: &PyObjectRef) {
        let ctx = self
            .exc_context_stack
            .last()
            .map(|(e, _)| e.clone())
            .or_else(|| self.propagating_exc.clone());
        if std::env::var("RPY_DEBUG_CTX").is_ok() {
            eprintln!(
                "CTX: exc={} stacklen={} ctx={:?}",
                exc.borrow().repr(),
                self.exc_context_stack.len(),
                ctx.as_ref().map(|c| c.borrow().repr())
            );
        }
        if let Some(ctx_exc) = ctx {
            // `raise e` re-raising the SAME object that is being handled does
            // not re-chain it as its own context (CPython checks
            // `exc_context != exc`). Without this, `except ZDE as e: raise e`
            // made `e.__context__ is e`.
            if !ctx_exc.is(exc) {
                // Some internal error paths construct exceptions as `imm()`
                // (immutable) — those can't be mutated; skip chaining.
                let mut borrow_guard = match exc.borrow_mut_if_mut() {
                    Some(g) => g,
                    None => {
                        self.propagating_exc = Some(exc.clone());
                        return;
                    }
                };
                let borrowed = &mut *borrow_guard;
                match borrowed {
                    PyObject::Exception {
                        context: ref mut ctx_field,
                        ..
                    } => {
                        *ctx_field = Some(ctx_exc.clone());
                    }
                    PyObject::Instance { dict, .. } => {
                        dict.insert_str("__context__", ctx_exc.clone());
                    }
                    _ => {}
                }
                drop(borrow_guard);
                // Break chaining cycles, exactly as CPython does
                // (`if (exc_context->context == exc) exc_context->context =
                // NULL`): `except A: try: raise B except B: raise a` would
                // otherwise leave a↔b circular (a.__context__=b,
                // b.__context__=a), which CPython's own
                // `test_raise::test_reraise_cycle_broken` asserts is broken.
                let mut ctx_guard = match ctx_exc.borrow_mut_if_mut() {
                    Some(g) => g,
                    None => {
                        self.propagating_exc = Some(exc.clone());
                        return;
                    }
                };
                match &mut *ctx_guard {
                    PyObject::Exception {
                        context: ref mut ctx_field,
                        ..
                    } => {
                        if let Some(c) = ctx_field {
                            if c.is(exc) {
                                *ctx_field = None;
                            }
                        }
                    }
                    PyObject::Instance { dict, .. } => {
                        if let Some(c) = dict.get_str("__context__") {
                            if c.is(exc) {
                                dict.insert_str("__context__", crate::object::py_none());
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        self.propagating_exc = Some(exc.clone());
    }

    /// Refresh the cached f_locals dict (`frame_locals_obj`) of live frame
    /// `idx` in place — same PyObject, updated contents. No-op when the
    /// frame has never handed out a frame object (the common case: frames
    /// nobody introspects pay nothing).
    pub(crate) fn sync_frame_locals(&mut self, idx: usize) {
        let Some(f) = self.frames.get(idx) else { return };
        let Some(obj) = &f.frame_locals_obj else { return };
        if f.code.varnames.is_empty() {
            return; // module frame: f_locals IS f_globals, already live
        }
        let obj = obj.clone();
        let f = self.frames.get(idx).unwrap();
        let pairs: Vec<(String, PyObjectRef)> = f
            .code
            .varnames
            .iter()
            .enumerate()
            .filter_map(|(i, name_id)| {
                f.fast_locals
                    .get(i)
                    .and_then(|slot| slot.as_ref())
                    .map(|v| (crate::interner::lookup_str(*name_id).to_string(), v.clone()))
            })
            .collect();
        if let PyObjectRef::Mut(rc) = &obj {
            if let Ok(mut b) = rc.try_borrow_mut() {
                if let PyObject::Dict(d) = &mut *b {
                    d.clear();
                    for (k, v) in pairs {
                        let _ = d.set(py_str(&k), v);
                    }
                }
            }
        }
    }

    /// The Python `frame` object for live frame `idx`, created once and
    /// cached on the Frame so `sys._getframe()` and a traceback's `tb_frame`
    /// return the SAME object (CPython asserts this identity in test_raise).
    pub(crate) fn frame_object(&mut self, idx: usize) -> Option<PyObjectRef> {
        if let Some(fo) = self.frames.get(idx).and_then(|f| f.frame_object.clone()) {
            if std::env::var("RPY_DEBUG_FRAME_OBJ").is_ok() {
                eprintln!("FRAME_OBJ reuse idx={} frames={}", idx, self.frames.len());
            }
            self.sync_frame_locals(idx);
            return Some(fo);
        }
        let frame = self.frames.get(idx)?;
        if std::env::var("RPY_DEBUG_FRAME_OBJ").is_ok() {
            eprintln!(
                "FRAME_OBJ create idx={} frames={} code={}",
                idx,
                self.frames.len(),
                self.frames[idx].code.name
            );
        }
        let code = frame.code.clone();
        let globals = frame.globals.clone();
        let builtins = frame.builtins.clone();
        let back_idx = frame.back;
        let f_lineno = {
            let idx = frame
                .ip
                .saturating_sub(1)
                .min(frame.code.instructions.len().saturating_sub(1));
            frame.code.line_number(idx) as i64
        };
        let f_lasti = frame.ip.saturating_sub(1) as i64;
        thread_local! {
            static BUILTINS_MIRROR: std::cell::RefCell<Option<(*const HashMap<StrId, PyObjectRef>, PyObjectRef)>> =
                const { std::cell::RefCell::new(None) };
        }
        let fb_obj: PyObjectRef = {
            let ptr = std::rc::Rc::as_ptr(&frame.builtins);
            BUILTINS_MIRROR.with(|m| {
                let mut m = m.borrow_mut();
                match &*m {
                    Some((p, obj)) if *p == ptr => obj.clone(),
                    _ => {
                        let mut fb = PyDict::new();
                        for (k, v) in builtins.iter() {
                            let _ =
                                fb.set(py_str(crate::interner::lookup_str(*k)), v.clone());
                        }
                        let obj = PyObjectRef::new(PyObject::Dict(Box::new(fb)));
                        *m = Some((ptr, obj.clone()));
                        obj
                    }
                }
            })
        };
        // f_locals: snapshot of the frame's local variables at frame-object
        // creation time. For function frames that's every varname slot that
        // currently holds a value (cellvar slots live in fast_locals too, as
        // LOAD_DEREF's own lookup confirms). For module/exec frames
        // (no varnames) CPython defines f_locals IS f_globals, so copy those.
        // f_globals: LIVE view over the frame's globals map
        // (PyObject::Globals — full dict protocol, O(1) to create). The old
        // code copied the whole map on EVERY frame-object creation, which
        // made introspection-heavy tests (mapping_tests builds 150k frames)
        // pay O(globals) each time and effectively hang. Module/exec frames
        // share this same object as f_locals (CPython identity rule).
        let fg_obj = PyObjectRef::imm(PyObject::Globals(globals.clone()));
        let fl_obj_owned;
        if !frame.code.varnames.is_empty() {
            let mut fl = PyDict::new();
            for (i, name_id) in frame.code.varnames.iter().enumerate() {
                if let Some(Some(v)) = frame.fast_locals.get(i) {
                    let _ = fl.set(py_str(crate::interner::lookup_str(*name_id)), v.clone());
                }
            }
            fl_obj_owned = PyObjectRef::new(PyObject::Dict(Box::new(fl)));
        } else {
            fl_obj_owned = fg_obj.clone();
        }
        let mut attrs = AttrMap::new();
        attrs.insert_str("f_globals", fg_obj);
        attrs.insert_str("f_builtins", fb_obj);
        let fl_obj = fl_obj_owned;
        attrs.insert_str("f_locals", fl_obj.clone());
        attrs.insert_str("f_code", PyObjectRef::imm(PyObject::Code(code)));
        attrs.insert_str("f_lineno", py_int(f_lineno));
        attrs.insert_str("f_lasti", py_int(f_lasti));
        // Tracing hooks and generator back-reference: present with inert
        // defaults so attribute access doesn't AttributeError (real tracing
        // support is a separate feature).
        attrs.insert_str("f_trace", py_none());
        attrs.insert_str("f_trace_lines", py_bool(true));
        attrs.insert_str("f_trace_opcodes", py_bool(false));
        attrs.insert_str("f_generator", py_none());
        // f_back: the previous frame in the call stack, or None
        if let Some(back_i) = back_idx {
            if let Some(back_frame_obj) = self.frame_object(back_i) {
                attrs.insert_str("f_back", back_frame_obj);
            } else {
                attrs.insert_str("f_back", py_none());
            }
        } else {
            attrs.insert_str("f_back", py_none());
        }
        let frame_obj = PyObjectRef::new(PyObject::Instance {
            typ: PyObjectRef::new(PyObject::Type {
                name: "frame".to_string(),
                dict: Box::new(TypeDict::default()),
                bases: vec![],
                mro: vec![],
            }),
            dict: attrs,
        });
        if let Some(f) = self.frames.get_mut(idx) {
            f.frame_object = Some(frame_obj.clone());
            f.frame_locals_obj = Some(fl_obj);
        }
        Some(frame_obj)
    }

    /// Build a `types.TracebackType` node for live frame `frame_idx` (a real
    /// frame object — the cached one, so `tb_frame is sys._getframe()` holds —
    /// plus lasti/lineno), for use as that frame's entry in a raised
    /// exception's `__traceback__` chain.
    fn make_traceback_node(&mut self, frame_idx: usize) -> Option<PyObjectRef> {
        let (line, lasti) = {
            let frame = self.frames.get(frame_idx)?;
            let idx = frame
                .ip
                .saturating_sub(1)
                .min(frame.code.instructions.len().saturating_sub(1));
            (frame.code.line_number(idx) as i64, idx as i64)
        };
        let frame_obj = self.frame_object(frame_idx)?;
        let tb_type = self
            .modules
            .get("types")
            .and_then(|t| t.borrow().get_attribute("TracebackType").ok())?;
        self.call_function(
            tb_type,
            vec![py_none(), frame_obj, py_int(lasti), py_int(line)],
            vec![],
        )
        .ok()
    }

    /// Prepend the current frame's traceback node to `exc`'s `__traceback__`
    /// chain (CPython builds the chain outermost-first, prepending a node per
    /// frame the exception unwinds through). Called on every frame an
    /// exception passes through, including the frame that finally catches it.
    pub(crate) fn prepend_traceback(&mut self, exc: &PyObjectRef, frame_idx: usize) {
        let Some(new_node) = self.make_traceback_node(frame_idx) else {
            return;
        };
        let old_tb = match &*exc.borrow() {
            PyObject::Exception {
                traceback: Some(tb),
                ..
            } => Some(tb.clone()),
            PyObject::Instance { dict, .. } => dict.get_str("__traceback__").cloned(),
            _ => None,
        };
        if let Some(old) = old_tb {
            let _ = new_node.borrow_mut().set_attribute("tb_next", old);
        }
        let mut guard = match exc.borrow_mut_if_mut() {
            Some(g) => g,
            None => return,
        };
        match &mut *guard {
            PyObject::Exception { traceback, .. } => {
                *traceback = Some(new_node);
            }
            PyObject::Instance { dict, .. } => {
                dict.insert_str("__traceback__", new_node);
            }
            _ => {}
        }
    }

    /// The real exception OBJECT a `PyError` represents — shared by
    /// `handle_exception` (pushes it for the handler/CHECK_EXC_MATCH to see)
    /// and `execute()`/`throw_into_frame` (need the same real object, not a
    /// bare string, to populate `exc_value`/derive `exc_type` for
    /// `sys.exc_info()` — see those call sites' own comments for the exact
    /// bug this fixes).
    pub(crate) fn error_to_exc_obj(error: &PyError) -> PyObjectRef {
        match error {
            // Reuse the original PyObjectRef exactly as raised — preserves
            // object identity (needed for `except E as e: ... raise` to
            // compare `e` against the handler-bound exception, and for
            // CPython's own `exc is value` idiom as used by contextlib's
            // generator-based context managers), plus its real
            // args/__cause__/extra attrs, instead of rebuilding a throwaway
            // single-message copy.
            //
            // EXCEPT for one ad hoc shape: a generator's own
            // `__next__`/`send`/`throw` driver (`object.rs`'s Generator
            // match arm) signals "generator returned instead of yielding
            // again" as `PyError::Exception("StopIteration".into(),
            // return_value)` — `return_value` there is the generator's raw
            // return value (often `None`), NOT a real exception object (see
            // `is_stop_iteration_error`'s doc comment, which already knows
            // to check the message string for exactly this reason). Pushing
            // that raw value as-is meant a Python-level `except
            // StopIteration as exc:` clause could never recognize it
            // (CHECK_EXC_MATCH has nothing exception-shaped to classify),
            // breaking `contextlib.contextmanager`'s own `__exit__`, which
            // relies on exactly that to detect a generator finishing in
            // response to `.throw()`. Build a real `StopIteration` instance
            // instead, carrying the return value as its arg (matching real
            // CPython's `StopIteration(value)`).
            PyError::Exception(msg, exc)
                if msg == "StopIteration"
                    && !matches!(&*exc.borrow(), PyObject::Exception { typ, .. } if typ == "StopIteration") =>
            {
                // Same `Mut`-not-`Imm` fix, same reason, as `synth_exception`
                // just above — a synthesized exception object must support
                // attribute assignment (`.__traceback__ = ...` etc.).
                PyObjectRef::new(PyObject::Exception {
                    typ: "StopIteration".to_string(),
                    args: vec![exc.clone()],
                    cause: None,
                    suppress_context: false,
                    context: None,
                    traceback: None,
                    extra: None,
                })
            }
            PyError::Exception(_, exc) => exc.clone(),
            PyError::TypeError(_) => Self::synth_exception("TypeError", error),
            PyError::ValueError(_) => Self::synth_exception("ValueError", error),
            PyError::NameError(_) => Self::synth_exception("NameError", error),
            PyError::AttributeError(_) => Self::synth_exception("AttributeError", error),
            PyError::IndexError(_) => Self::synth_exception("IndexError", error),
            PyError::KeyError(_) => Self::synth_exception("KeyError", error),
            PyError::ZeroDivisionError(_) => Self::synth_exception("ZeroDivisionError", error),
            PyError::RuntimeError(_) => Self::synth_exception("RuntimeError", error),
            PyError::SystemExit(code) => PyObjectRef::new(PyObject::Exception {
                typ: "SystemExit".to_string(),
                args: vec![py_int(*code as i64)],
                cause: None,
                suppress_context: false,
                context: None,
                traceback: None,
                extra: None,
            }),
            PyError::StopIteration => Self::synth_exception("StopIteration", error),
            PyError::ImportError(_) => Self::synth_exception("ImportError", error),
            PyError::RecursionError(_) => Self::synth_exception("RecursionError", error),
            // `PyError::OsError` (raised by essentially every file/OS
            // operation — `os.stat`/`open()`/`read()`/`write()`/etc. — for
            // any underlying `std::io::Error`) previously fell through to
            // the generic `_` catch-all below, synthesizing a bare
            // `Exception` instead of a real, catchable `OSError`. Broke the
            // extremely common `try: os.stat(path) except OSError:`
            // existence-check idiom (used throughout the real stdlib
            // itself — real trigger: vendoring `dbm/__init__.py`'s own
            // `whichdb()`, `except OSError:` around a missing-file
            // `os.stat()` call) and any other OS-error-handling code
            // anywhere in the ecosystem.
            PyError::OsError(_) => Self::synth_exception("OSError", error),
            _ => Self::synth_exception("Exception", error),
        }
    }

    pub(crate) fn handle_exception(&mut self, error: &PyError, frame_floor: usize) -> bool {
        // Only this execute_inner invocation's own frame may handle the
        // exception here — frames below `frame_floor` belong to an outer,
        // suspended execute() call and must never be touched from inside a
        // nested one (see the comment on `execute()` for why).
        // The exception object being unwound. Capture its implicit
        // __context__ BEFORE any handler's context-stack truncation below:
        // the stack still holds the handlers this exception is being raised
        // inside (e.g. `except ValueError: 1/0` — the ZeroDivisionError from
        // the division builtin chains ValueError even though no `raise`
        // statement produced it). Done before the loop because it borrows all
        // of `&mut self` (method call), which cannot coexist with the
        // per-frame mutable borrow in the loop.
        let exc_obj = Self::error_to_exc_obj(error);
        self.capture_exception_context(&exc_obj);
        // Set the current exception info so `sys.exc_info()` inside the
        // handler returns the REAL caught exception (type, value, and the
        // traceback we're about to attach). Previously only RAISE_VARARGS
        // populated these, so a builtin-raised error (`[1,2].blah`,
        // `1/0`, ...) caught by `except E as e:` made `sys.exc_info()`
        // return (None, None, None) — breaking traceback.format_exception,
        // unittest's error reporting, and `except E as e: sys.exc_info()`
        // introspection.
        self.exc_type = Some(self.exception_class_of(&exc_obj));
        self.exc_value = Some(exc_obj.clone());
        let total = self.frames.len();
        for i in (frame_floor..total).rev() {
            // Pop the innermost handler of this frame and set up control
            // transfer WITHOUT holding the frame borrow across
            // `prepend_traceback` (which needs `&mut self`).
            let (entered, orig_ip, handler_addr) = {
                let frame = &mut self.frames[i];
                match frame.exception_handlers.pop() {
                    Some(handler) => {
                        let orig = frame.ip;
                        // For any handler (Except or Finally), restore stack and
                        // transfer control.
                        frame.stack.truncate(handler.stack_depth);
                        // Drop context-stack entries whose handled exception's value
                        // was above this truncation point: their handler bodies were
                        // abandoned mid-execution by the exception now unwinding (its
                        // POP_EXCEPT epilogue never ran), so they must not linger and
                        // pollute a later unrelated raise's __context__.
                        self.exc_context_stack
                            .retain(|(_, depth)| *depth < handler.stack_depth);
                        frame.ip = handler.instr_addr;
                        (true, orig, handler.instr_addr)
                    }
                    None => (false, 0, 0),
                }
            };
            if entered {
                // Prepend a real traceback node for this catching frame using
                // the SAME exception object the handler will bind — for
                // synthesized PyErrors (TypeError/AttributeError/...) every
                // `error_to_exc_obj` call makes a FRESH object, so doing the
                // prepend here (not in execute()'s else-branch) is what makes
                // `except E as e: e.__traceback__` non-None.
                // Use the ORIGINAL ip (where the exception was raised) for the
                // traceback's lineno, not the handler's ip. The handler's line
                // (e.g. `return e` inside `except`) is not where the exception
                // occurred — the `with` or `for` line is. Using the handler's
                // ip made `test_with::testExceptionLocation` report 822 (the
                // `return e` line) instead of 819 (the `with` line), and made
                // `test_dictcomps` similarly off before the `first_lineno` fix.
                let saved_handler_ip = handler_addr;
                self.frames[i].ip = orig_ip;
                self.prepend_traceback(&exc_obj, i);
                self.frames[i].ip = saved_handler_ip;
                // The unwinding exception is now "in flight" — a `raise X`
                // inside a `finally:` body chains it as __context__ (except
                // handlers' PUSH_EXC_INFO immediately replaces this with the
                // handled-exception stack entry, which is the CPython
                // semantics: the handled exception becomes the context).
                self.propagating_exc = Some(exc_obj.clone());
                self.frames[i].push(exc_obj);
                // For Finally handlers, we always execute them.
                // For Except handlers, we also execute them — the code at the
                // handler address will check CHECK_EXC_MATCH to decide.
                // The key difference: after a Finally handler finishes, the
                // exception is re-raised via RERAISE (by the code the compiler
                // emits after the finally block). After an Except handler
                // finishes, there's no RERAISE — the exception was handled.
                return true;
            }
        }
        // No handler in this execute()'s own frames. The exception keeps
        // propagating as Err to the Rust caller (an enclosing execute() may
        // still catch it — e.g. an exception raised inside a function called
        // from a handler, `def f(): raise; except E: f()`). Only when this is
        // the OUTERMOST execute call (frame_floor == 0) and nothing caught
        // it is it truly uncaught, and only then is the propagating/context
        // state reset so nothing stale chains into the NEXT raise.
        if frame_floor == 0 {
            self.exc_context_stack.clear();
            self.propagating_exc = None;
        }
        false
    }
}
