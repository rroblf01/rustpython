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
    /// True if `val` is literally one of the objects stored in a registered
    /// module's (or the `builtins` namespace's) dict under `name`. In real
    /// CPython such plain builtin functions (`math.isclose`, `random.random`,
    /// ...) have no `__get__` and are NOT descriptors — so when a Python
    /// class body copies one (`isclose = math.isclose`) and it is later
    /// accessed through an instance, it must be returned unbound. Genuine
    /// native METHODS (built into a type's own dict, e.g. `hmac.HMAC.hexdigest`)
    /// never appear in any module namespace, so they still auto-bind as
    /// before.
    pub(crate) fn is_plain_module_function(&self, name: &str, val: &PyObjectRef) -> bool {
        let addr = &*val.borrow() as *const PyObject as usize;
        let scan = |dict: &crate::object::TypeDict| -> bool {
            match dict.get_str(name) {
                Some(v) => &*v.borrow() as *const PyObject as usize == addr,
                None => false,
            }
        };
        for (_, m) in self.modules.iter() {
            if let crate::object::PyObject::Module { dict, .. } = &*m.borrow() {
                if scan(dict) {
                    return true;
                }
            }
        }
        for (_, v) in self.builtins.iter() {
            if let crate::object::PyObject::Module { dict, .. } = &*v.borrow() {
                if scan(dict) {
                    return true;
                }
            }
            if &*v.borrow() as *const PyObject as usize == addr {
                return true;
            }
        }
        false
    }

    /// Resolves `name` on an `Instance` object via its type/MRO (NOT its own
    /// `__dict__` — callers check that themselves first, matching instance-
    /// dict-over-non-data-descriptor precedence), applying the full
    /// descriptor protocol: `property` getters, `staticmethod`/`classmethod`
    /// unwrapping/binding, plain-function-to-bound-method binding, and a
    /// generic `__get__` call for any other descriptor. This mirrors LOAD_ATTR's
    /// own inline logic (kept separate/duplicated rather than shared, to avoid
    /// touching that hot, delicate opcode path) — used by `getattr()`'s
    /// special-case below so it stops returning raw, un-invoked descriptors
    /// (confirmed general: `getattr(obj, 'some_property')` returned the
    /// `property` object itself instead of calling its getter).
    pub(crate) fn resolve_descriptor_attr(
        &mut self,
        obj: &PyObjectRef,
        name: &str,
    ) -> Option<PyObjectRef> {
        let typ = if let PyObject::Instance { typ, .. } = &*obj.borrow() {
            typ.clone()
        } else {
            return None;
        };
        let found = {
            let typ_ref = typ.borrow();
            if let PyObject::Type {
                dict: type_dict,
                mro,
                ..
            } = &*typ_ref
            {
                type_dict.get_str(name).cloned().or_else(|| {
                    for base in mro.iter().skip(1) {
                        if let PyObject::Type {
                            dict: base_dict, ..
                        } = &*base.borrow()
                        {
                            if let Some(val) = base_dict.get_str(name) {
                                return Some(val.clone());
                            }
                        }
                    }
                    None
                })
            } else {
                None
            }
        }?;
        let val_borrowed = found.borrow();
        match &*val_borrowed {
            PyObject::Property(ref d) if d.getter.is_some() => {
                let g = d.getter.clone().unwrap();
                drop(val_borrowed);
                Some(
                    self.call_function(g, vec![obj.clone()], vec![])
                        .unwrap_or_else(|_| found.clone()),
                )
            }
            PyObject::StaticMethod { func } => Some(func.clone()),
            PyObject::ClassMethod { func } => {
                let func_clone = func.clone();
                Some(PyObjectRef::imm(PyObject::BoundMethod {
                    func: func_clone,
                    self_obj: typ.clone(),
                }))
            }
            PyObject::Function(_) => Some(PyObjectRef::imm(PyObject::BoundMethod {
                func: found.clone(),
                self_obj: obj.clone(),
            })),
            PyObject::BuiltinFunction { name: n, .. }
                if crate::object::is_builtin_exception_class_name(n) =>
            {
                Some(found.clone())
            }
            PyObject::BuiltinFunction { name: n, func } => {
                Some(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: n.clone(),
                    func: *func,
                    self_obj: obj.clone(),
                }))
            }
            PyObject::BuiltinMethod { name: n, func, .. } => {
                Some(PyObjectRef::imm(PyObject::BuiltinMethod {
                    name: n.clone(),
                    func: *func,
                    self_obj: obj.clone(),
                }))
            }
            _ => {
                drop(val_borrowed);
                if let Ok(get_fn) = found.borrow().get_attribute("__get__") {
                    let descriptor_args = vec![found.clone(), obj.clone(), typ.clone()];
                    match self.call_function(get_fn, descriptor_args, vec![]) {
                        Ok(v) => Some(v),
                        Err(_) => Some(found.clone()),
                    }
                } else {
                    Some(found.clone())
                }
            }
        }
    }
}
