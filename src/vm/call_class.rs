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
        let original_bases_vec = if matches!(&*bases.borrow(), PyObject::None) {
            vec![]
        } else if let PyObject::Tuple(t) = &*bases.borrow() {
            t.clone()
        } else {
            vec![bases.clone()]
        };

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
                        let orig_bases_tuple =
                            PyObjectRef::new(PyObject::Tuple(original_bases_vec.clone()));
                        let result = {
                            let is_one_arg = match &*f.borrow() {
                                PyObject::Function(func) => func.code.arg_count == 1,
                                PyObject::BoundMethod { func, .. } => match &*func.borrow() {
                                    PyObject::Function(func) => func.code.arg_count == 1,
                                    _ => false,
                                },
                                _ => false,
                            };
                            if is_one_arg {
                                let inner = match &*f.borrow() {
                                    PyObject::BoundMethod { func, .. } => func.clone(),
                                    _ => f.clone(),
                                };
                                crate::object::call_function_disposable(
                                    &inner,
                                    vec![orig_bases_tuple],
                                    vec![],
                                )?
                            } else {
                                crate::object::call_bound_method(
                                    f,
                                    base.clone(),
                                    vec![orig_bases_tuple],
                                )?
                            }
                        };
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
                                    return Err(PyError::type_error("__setattr__ needs 3 args"));
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

        // `__prepare__(name, bases_tuple, **kwds)` — real CPython always
        // passes bases as a TUPLE (empty `()` when there are none, never
        // None or a bare base) and forwards every class keyword except
        // `metaclass` itself.
        let prepare_bases = if original_bases_vec.is_empty() {
            PyObjectRef::new(PyObject::Tuple(vec![]))
        } else {
            PyObjectRef::new(PyObject::Tuple(bases_vec.clone()))
        };
        let prepared_namespace: Option<PyObjectRef> = if let Some(mc) = &effective_metaclass {
            // A non-type metaclass (plain function) can still carry a
            // `__prepare__` in its own `__dict__` (`meta.__prepare__ =
            // prepare`) — the MRO dunder walk only covers real types.
            let prep_lookup =
                crate::object::lookup_dunder_via_mro(mc, "__prepare__").or_else(|| {
                    if matches!(&*mc.borrow(), PyObject::Type { .. }) {
                        None
                    } else {
                        mc.borrow().get_attribute("__prepare__").ok()
                    }
                });
            match prep_lookup {
                Some(prep_fn) => {
                    // A descriptor instance (not directly callable) must be
                    // bound via `__get__(None, metaclass)` exactly like
                    // `type.__getattribute__` does — e.g. FailDescr whose
                    // `__get__` raises must fail class creation (never be
                    // silently swallowed into a default namespace).
                    let prep_fn = match &*prep_fn.borrow() {
                        PyObject::Function(_)
                        | PyObject::BuiltinFunction { .. }
                        | PyObject::BuiltinMethod { .. }
                        | PyObject::BoundMethod { .. }
                        | PyObject::Closure(_)
                        | PyObject::Type { .. }
                        | PyObject::StaticMethod { .. }
                        | PyObject::ClassMethod { .. } => prep_fn.clone(),
                        _ => match prep_fn.borrow().get_attribute("__get__") {
                            Ok(get) => self.call_function(
                                get,
                                vec![prep_fn.clone(), py_none(), mc.clone()],
                                vec![],
                            )?,
                            Err(_) => prep_fn.clone(),
                        },
                    };
                    let unwrapped = match &*prep_fn.borrow() {
                        PyObject::ClassMethod { func } => func.clone(),
                        PyObject::StaticMethod { func } => func.clone(),
                        _ => prep_fn.clone(),
                    };
                    let is_classmethod = matches!(&*prep_fn.borrow(), PyObject::ClassMethod { .. });
                    let call_args = if is_classmethod {
                        vec![mc.clone(), name.clone(), prepare_bases.clone()]
                    } else {
                        vec![name.clone(), prepare_bases.clone()]
                    };
                    Some(self.call_function(unwrapped, call_args, init_subclass_kwargs.clone())?)
                }
                None => None,
            }
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
                // Route the body directly against a custom `__prepare__`
                // mapping (see `Frame::prepared_namespace`): every
                // STORE_NAME goes through its `__setitem__` as it happens
                // and LOAD_NAME reads pre-populated entries.
                new_frame.prepared_namespace = prepared_namespace.clone();
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

        let mut namespace_dict: HashMap<String, PyObjectRef> = namespace
            .borrow()
            .iter()
            .map(|(k, v)| (interner::lookup_str(*k).to_string(), v.clone()))
            .collect();
        // PEP 560: store original bases as __orig_bases__ before resolution
        // so typing.py's __init_subclass__ can distinguish Generic[T] from Generic.
        if !original_bases_vec.is_empty() {
            namespace_dict.insert(
                "__orig_bases__".to_string(),
                PyObjectRef::new(PyObject::Tuple(original_bases_vec.clone())),
            );
        }
        let order = name_order.borrow().clone();

        // NOTE: no post-execution replay into `prepared` is needed (or
        // wanted) anymore: with `Frame::prepared_namespace` set above,
        // every body STORE_NAME already went through its `__setitem__`
        // AS IT HAPPENED (and reads saw pre-populated entries). Replaying
        // here would apply every store a SECOND time — observably wrong
        // for instrumented-dict namespaces. The old replay block lived
        // here and is intentionally gone; `prepared` itself (holding all
        // body stores plus its pre-populated content) is what
        // `build_class_with_metaclass` hands to `__new__` below.

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
                // Same tuple the `__prepare__` call above received:
                // resolved bases, empty when none were written.
                prepare_bases.clone(),
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
