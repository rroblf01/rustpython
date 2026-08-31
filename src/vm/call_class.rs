use crate::interner::{self, StrId};
use crate::object::*;
use crate::vm::VirtualMachine;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

impl VirtualMachine {
    pub(crate) fn handle_build_class(
        &mut self,
        args: Vec<PyObjectRef>,
        keywords: Vec<(String, PyObjectRef)>,
    ) -> PyResult<PyObjectRef> {
        if args.len() < 3 {
            return Err(PyError::type_error(
                "__build_class__: need at least 3 arguments",
            ));
        }
        let func = args[0].clone();
        let name = args[1].clone();
        let bases = args[2].clone();

        let name_str = match &*name.borrow() {
            PyObject::Str(s) => s.to_string(),
            _ => return Err(PyError::type_error("class name must be a string")),
        };

        let explicit_metaclass = keywords
            .iter()
            .find(|(k, _)| k == "metaclass")
            .map(|(_, v)| v.clone());

        let bases_vec = if matches!(&*bases.borrow(), PyObject::None) {
            vec![]
        } else if let PyObject::Tuple(t) = &*bases.borrow() {
            t.clone()
        } else {
            vec![bases.clone()]
        };
        // PEP 560: resolve non-class bases via `__mro_entries__` before
        // actual class creation runs — real trigger: `class ThemeSection
        // (Mapping[str, str]): ...` (real CPython's own
        // `Lib/_colorize.py`, pulled in transitively by `unittest` once
        // `collections.abc` stopped being a hand-rolled native module and
        // became the real, vendored `Lib/_collections_abc.py`).
        // `Mapping[str, str]` is a `types.GenericAlias` INSTANCE (built by
        // `Mapping.__class_getitem__`), not a class — used directly as a
        // base it previously fell straight through to ordinary class
        // creation, which requires each base to be an actual class,
        // raising a confusing `AttributeError: 'super' object has no
        // attribute '__new__'` from deep inside the metaclass chain rather
        // than ever substituting the real origin class. Mirrors real
        // CPython's own `types.resolve_bases`: a base that ISN'T a class
        // but a defines `__mro_entries__` gets replaced (possibly with
        // MULTIPLE entries) by the result of calling it with the full
        // original bases tuple; anything else (an ordinary class, or a
        // base with no such hook) passes through unchanged.
        let bases_vec: Vec<PyObjectRef> = {
            let mut resolved = Vec::with_capacity(bases_vec.len());
            for base in &bases_vec {
                let is_class_like = matches!(
                    &*base.borrow(),
                    PyObject::Type { .. } | PyObject::BuiltinFunction { .. }
                );
                if is_class_like {
                    resolved.push(base.clone());
                    continue;
                }
                let mro_entries_fn = base.borrow().get_attribute("__mro_entries__").ok();
                match mro_entries_fn {
                    Some(f) => {
                        let result =
                            crate::object::call_bound_method(f, base.clone(), vec![bases.clone()])?;
                        let entries = match &*result.borrow() {
                            PyObject::Tuple(items) => items.clone(),
                            _ => {
                                return Err(PyError::type_error(
                                    "__mro_entries__ must return a tuple",
                                ));
                            }
                        };
                        resolved.extend(entries);
                    }
                    None => resolved.push(base.clone()),
                }
            }
            resolved
        };
        let bases_vec = if bases_vec.is_empty() {
            let object_type = self
                .builtins
                .get(&interner::intern("object"))
                .cloned()
                .unwrap_or_else(|| {
                    let mut obj_dict: TypeDict = Default::default();
                    obj_dict.insert_str(
                        "__setattr__",
                        PyObjectRef::new(PyObject::BuiltinFunction {
                            name: "__setattr__".to_string(),
                            func: |args| {
                                if args.len() < 3 {
                                    return Err(PyError::type_error(
                                        "__setattr__ needs 3 args",
                                    ));
                                }
                                args[0]
                                    .borrow_mut()
                                    .set_attribute(&args[1].str(), args[2].clone())?;
                                Ok(py_none())
                            },
                        }),
                    );
                    PyObjectRef::new(PyObject::Type {
                        name: "object".to_string(),
                        dict: Box::new(obj_dict),
                        bases: vec![],
                        mro: vec![],
                    })
                });
            vec![object_type]
        } else {
            bases_vec
        };

        let init_subclass_kwargs: Vec<(String, PyObjectRef)> = keywords
            .iter()
            .filter(|(k, _)| k != "metaclass")
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        let inherited_metaclass = bases_vec.iter().find_map(crate::object::metatype_of);
        let effective_metaclass = explicit_metaclass.or(inherited_metaclass);

        let prepared_namespace: Option<PyObjectRef> = if let Some(mc) = &effective_metaclass {
            crate::object::lookup_dunder_via_mro(mc, "__prepare__").and_then(|prep_fn| {
                let unwrapped = match &*prep_fn.borrow() {
                    PyObject::ClassMethod { func } => func.clone(),
                    PyObject::StaticMethod { func } => func.clone(),
                    _ => prep_fn.clone(),
                };
                let is_classmethod = matches!(&*prep_fn.borrow(), PyObject::ClassMethod { .. });
                let call_args = if is_classmethod {
                    vec![mc.clone(), name.clone(), bases.clone()]
                } else {
                    vec![name.clone(), bases.clone()]
                };
                self.call_function(unwrapped, call_args, vec![]).ok()
            })
        } else {
            None
        };

        let namespace: Rc<RefCell<HashMap<StrId, PyObjectRef>>> =
            Rc::new(RefCell::new(HashMap::new()));
        let name_order = Rc::new(RefCell::new(Vec::new()));

        let caller_module_globals = if self.frames.len() >= 1 {
            let caller_frame = &self.frames[self.frames.len() - 1];
            caller_frame
                .module_globals
                .clone()
                .or_else(|| Some(caller_frame.globals.clone()))
        } else {
            None
        };

        let mut class_cell: Option<PyObjectRef> = None;
        match &*func.borrow() {
            PyObject::Function(ref f) => {
                let code = &f.code;
                let closure = &f.closure;
                let code = code.clone();
                let closure = closure.clone();
                let mut new_frame = self.acquire_frame(
                    code,
                    namespace.clone(),
                    Rc::clone(&self.builtins),
                    caller_module_globals,
                );
                new_frame.closure = Box::new(closure);
                new_frame.name_order = Some(name_order.clone());
                self.push_frame(new_frame);
                let result = self.execute();
                class_cell = {
                    let popped = self.frames.pop();
                    let cell = popped.as_ref().and_then(|fr| {
                        let idx = fr
                            .code
                            .varnames
                            .iter()
                            .position(|&n| crate::interner::lookup_str(n) == "__class__");
                        idx.and_then(|i| fr.fast_locals.get(i).and_then(|v| v.clone()))
                    });
                    if let Some(frame) = popped {
                        self.release_frame(frame);
                    }
                    cell
                };
                result?;
            }
            _ => return Err(PyError::type_error("class body must be a function")),
        }

        let namespace_dict: HashMap<String, PyObjectRef> = namespace
            .borrow()
            .iter()
            .map(|(k, v)| (interner::lookup_str(*k).to_string(), v.clone()))
            .collect();
        let order = name_order.borrow().clone();

        if let Some(prepared) = &prepared_namespace {
            let setitem_fn = if let PyObject::Instance { typ, .. } = &*prepared.borrow() {
                crate::object::lookup_dunder_via_mro(typ, "__setitem__")
            } else {
                None
            };
            if std::env::var("RPY_DEBUG_METACLASS").is_ok() {
                eprintln!(
                    "prepare-replay: name={} order={:?} has_setitem={}",
                    name_str,
                    order,
                    setitem_fn.is_some()
                );
            }
            for k in &order {
                if let Some(v) = namespace_dict.get(k) {
                    if std::env::var("RPY_DEBUG_METACLASS").is_ok() {
                        eprintln!("  replaying key={} value={}", k, v.repr());
                    }
                    if let Some(f) = &setitem_fn {
                        self.call_function(
                            f.clone(),
                            vec![prepared.clone(), py_str(k), v.clone()],
                            vec![],
                        )?;
                    } else if let Some(native) = crate::object::native_backing_of(prepared) {
                        if let PyObject::Dict(pd) = &mut *native.borrow_mut() {
                            pd.set(py_str(k), v.clone())?;
                        }
                    }
                }
            }
            if std::env::var("RPY_DEBUG_METACLASS").is_ok() {
                if let Some(native) = crate::object::native_backing_of(prepared) {
                    if let PyObject::Dict(pd) = &*native.borrow() {
                        eprintln!(
                            "  final native dict keys: {:?}",
                            pd.keys().iter().map(|k| k.str()).collect::<Vec<_>>()
                        );
                    }
                }
            }
        }

        let class_result = if let Some(mc) = effective_metaclass {
            self.build_class_with_metaclass(
                name_str,
                name.clone(),
                bases_vec,
                namespace_dict,
                order,
                mc,
                init_subclass_kwargs,
                prepared_namespace,
            )
        } else {
            self.default_build_class(
                name_str,
                bases_vec,
                namespace_dict,
                init_subclass_kwargs,
                None,
            )
        };
        let class_obj = class_result?;
        if let Some(cell) = class_cell {
            if let PyObject::Cell { value } = &mut *cell.borrow_mut() {
                *value = Some(class_obj.clone());
            }
        }
        Ok(class_obj)
    }
}
