// Auto-extracted from src/object/attrs/mod.rs lines 4190-4228
use super::*;
use crate::interner;
use crate::object::*;

pub(crate) fn get(o: &PyObject, name: &str) -> PyResult<PyObjectRef> {
    match o {
        PyObject::Generator { frame: gen_frame } => match name {
            "__next__" | "send" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                name: name.to_string(),
                func: generator_next_fallback,
                self_obj: PyObjectRef::new(PyObject::None),
            })),
            "throw" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                name: "throw".to_string(),
                func: generator_throw_fallback,
                self_obj: PyObjectRef::new(PyObject::None),
            })),
            "close" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                name: "close".to_string(),
                func: |args| {
                    let gen = args[0].borrow();
                    if let PyObject::Generator { frame } = &*gen {
                        if let Ok(mut frame_opt) = frame.try_borrow_mut() {
                            *frame_opt = None;
                        }
                    }
                    Ok(py_none())
                },
                self_obj: PyObjectRef::new(PyObject::None),
            })),
            "__iter__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                name: "__iter__".to_string(),
                func: |args| Ok(args[0].clone()),
                self_obj: PyObjectRef::new(PyObject::None),
            })),
            "__await__" => Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                name: "__await__".to_string(),
                func: |args| Ok(args[0].clone()),
                self_obj: PyObjectRef::new(PyObject::None),
            })),
            // `gi_running` is True (real CPython actually exposes it as
            // a plain int 0/1, not bool — confirmed via CPython's own
            // `test_genexps.py` doctest, which prints bare `0`/`1`, not
            // `False`/`True`) exactly while this generator's OWN frame
            // is mid-execution — which, conveniently, is exactly what
            // `generator_next_fallback`'s reentrancy guard already
            // tracks via `RefCell::try_borrow_mut` on this same `frame`
            // cell (a concurrent/reentrant `next()` on a running
            // generator fails that borrow with real CPython's
            // `ValueError: generator already executing`). A plain,
            // momentary `try_borrow()` here reuses that exact signal:
            // it fails (already mutably borrowed) if and only if this
            // generator is currently executing somewhere up the call
            // stack — including the exact case the doctest exercises,
            // `me.gi_running` evaluated as the generator's own yielded
            // expression while `next(me)` holds that borrow.
            "gi_running" => Ok(py_int(if gen_frame.try_borrow().is_ok() { 0 } else { 1 })),
            // Real CPython's `gi_frame` is a live frame object while
            // suspended, and `None` once the generator is exhausted.
            // This codebase has no general standalone frame-object
            // representation for a generator's OWN (not currently on
            // the VM's frame stack) saved frame outside of `sys.
            // _getframe`'s live-call-stack path (see that function's own
            // doc comment) — building one is a larger, separate gap.
            // Reporting plain `None` here is a known partial stub: it
            // satisfies `gi_frame` being a real, gettable attribute
            // (needed for `dir(g)` to include it, per CPython's own
            // `test_genexps.py` doctest) without claiming to support
            // introspecting the frame's own contents.
            "gi_frame" => Ok(py_none()),
            _ => Err(PyError::attribute_error(format!(
                "'generator' object has no attribute '{}'",
                name
            ))),
        },
        _ => unreachable!(
            "{} called with non-{} object",
            stringify!(get),
            o.type_name()
        ),
    }
}
