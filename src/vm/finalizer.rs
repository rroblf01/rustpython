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
    /// Execute queued `__del__` finalizers left by the last cycle-collector pass.
    /// Runs with the live `&mut VirtualMachine`; converts escaping exceptions
    /// into `sys.unraisablehook` calls — never propagates into caller bytecode.
    pub(crate) fn run_pending_finalizers(&mut self) {
        use crate::object::{ObjectAccess, VM_PTR};
        fn with_pinned_vm<R>(vm: *mut VirtualMachine, f: impl FnOnce() -> R) -> R {
            let prev = VM_PTR.get().or(Some(vm));
            VM_PTR.set(Some(vm));
            let out = f();
            VM_PTR.set(prev);
            out
        }
        let self_ptr: *mut VirtualMachine = self;
        let pending = crate::cycle_gc::take_pending_finalizers();
        let saved_active = self
            .frames
            .last()
            .and_then(|f| f.active_exception.clone());
        let saved_ctx_stack = std::mem::take(&mut self.exc_context_stack);
        let saved_propagating = self.propagating_exc.take();
        for (self_obj, del_fn) in pending {
            crate::cycle_gc::IN_FINALIZER.with(|f| f.set(true));
            let bound = PyObjectRef::imm(PyObject::BoundMethod {
                func: del_fn,
                self_obj: self_obj.clone(),
            });
            let result =
                with_pinned_vm(self_ptr, || self.call_function(bound, vec![], vec![]));
            crate::cycle_gc::IN_FINALIZER.with(|f| f.set(false));
            if let Err(err) = result {
                let carrier_typ = PyObjectRef::new(PyObject::Type {
                    name: "UnraisableHookArgs".into(),
                    dict: Box::new(crate::object::TypeDict::default()),
                    bases: vec![],
                    mro: vec![],
                });
                let exc_type_obj = {
                    let name = err.type_name();
                    let cls_name: Option<String> = match &err {
                        PyError::Exception(_, exc_obj) => ObjectAccess::get_attribute(
                            &*exc_obj.borrow(),
                            "__class__",
                        )
                        .ok()
                        .map(|c| c.borrow().type_name()),
                        _ => None,
                    };
                    let lookup = cls_name.as_deref().unwrap_or(name);
                    self.builtins
                        .get(&interner::intern(lookup))
                        .cloned()
                        .unwrap_or_else(|| py_str(name))
                };
                let mut dict = AttrMap::new();
                dict.insert_str("exc_type", exc_type_obj);
                dict.insert_str("exc_value", py_str(&err.message()));
                dict.insert_str("object", self_obj);
                dict.insert_str("truncated", py_bool(false));
                dict.insert_str("traceback", py_none());
                let carrier =
                    PyObjectRef::new(PyObject::Instance { typ: carrier_typ, dict });
                let hook = self
                    .modules
                    .get("sys")
                    .and_then(|m| ObjectAccess::get_attribute(&*m.borrow(), "unraisablehook").ok());
                match hook {
                    Some(hook_obj) => {
                        let _ = with_pinned_vm(self_ptr, || {
                            self.call_function(hook_obj, vec![carrier], vec![])
                        });
                    }
                    None => eprintln!(
                        "Exception ignored in __del__: {}: {}",
                        err.type_name(),
                        err.message()
                    ),
                }
            }
        }
        if let Some(top) = self.frames.last_mut() {
            top.active_exception = saved_active.clone();
        }
        self.exc_context_stack = saved_ctx_stack;
        self.propagating_exc = saved_propagating;
    }
}
