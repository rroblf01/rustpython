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

/// C3 linearization for proper method resolution order (MRO).
///
/// Implements the C3 algorithm used by CPython since Python 2.3.
/// Merges the MROs of all bases together with the direct bases list.
/// Returns an error if a consistent MRO cannot be created.
fn c3_linearize(bases: &[PyObjectRef]) -> PyResult<Vec<PyObjectRef>> {
    if bases.is_empty() {
        return Ok(vec![]);
    }

    // Build the lists to merge:
    // For each base, get its linearized MRO (already computed since classes
    // are created in dependency order). If the base's MRO is empty (as with
    // built-in types whose MRO was never computed), treat it as just [base].
    // The C3 algorithm also includes the direct bases list as the last merge
    // list to enforce base ordering constraints.
    let mut lists: Vec<Vec<PyObjectRef>> = Vec::new();
    for base in bases {
        let base_mro = if let PyObject::Type { mro, .. } = &*base.borrow() {
            if mro.is_empty() {
                vec![base.clone()]
            } else {
                mro.clone()
            }
        } else {
            vec![base.clone()]
        };
        lists.push(base_mro);
    }
    // Add the direct bases list as the final merge constraint (C3 spec)
    lists.push(bases.to_vec());

    let mut result: Vec<PyObjectRef> = Vec::new();
    loop {
        // Collect non-empty lists
        let non_empty: Vec<&Vec<PyObjectRef>> = lists.iter().filter(|l| !l.is_empty()).collect();
        if non_empty.is_empty() {
            return Ok(result);
        }

        let mut found = false;
        'candidate: for list in &non_empty {
            let candidate = &list[0];

            // Check if candidate appears in the tail of any other non-empty list
            for other in &non_empty {
                if other.len() > 1 {
                    for item in &other[1..] {
                        if item.is(candidate) {
                            continue 'candidate;
                        }
                    }
                }
            }

            // Candidate is valid — add to result and remove from all heads
            result.push(candidate.clone());
            // Clone before mutable borrow to break borrow checker conflict
            let candidate_clone = candidate.clone();
            for list in &mut lists {
                if !list.is_empty() && list[0].is(&candidate_clone) {
                    list.remove(0);
                }
            }
            found = true;
            break;
        }

        if !found {
            return Err(PyError::type_error(
                "Cannot create a consistent method resolution order (MRO)",
            ));
        }
    }
}

impl VirtualMachine {
    /// Real implementation behind `type.__new__(metacls, name, bases,
    /// namespace, **kwds)`, called directly from `call_function` (see the
    /// special-case there) with genuine `&mut self` access — mirrors
    /// `crate::object::type_new_builtin`'s argument parsing exactly, but
    /// without needing `with_vm_mut`'s thread-local re-entrant VM lookup.
    pub(crate) fn type_new_impl(&mut self, args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
        if args.len() < 4 {
            return Err(PyError::type_error(
                "type.__new__() takes at least 4 arguments (metacls, name, bases, namespace)",
            ));
        }
        if std::env::var("RPY_DEBUG_METACLASS").is_ok() {
            eprintln!(
                "type_new_impl: args={:?}",
                args.iter()
                    .map(|a| format!("{}:{}", a.get_type_name(), a.repr()))
                    .collect::<Vec<_>>()
            );
        }
        let metacls = args[0].clone();
        let name_str = args[1].str();
        let bases_vec = match &*args[2].borrow() {
            PyObject::Tuple(t) => t.clone(),
            PyObject::None => vec![],
            _ => vec![args[2].clone()],
        };
        let namespace_dict = crate::object::dict_arg_to_hashmap(
            &args[3],
            "type.__new__(): namespace must be a dict",
        )?;
        let kwargs: Vec<(String, PyObjectRef)> = match args.get(4) {
            Some(d) => match &*d.borrow() {
                PyObject::Dict(d) => d.items().into_iter().map(|(k, v)| (k.str(), v)).collect(),
                _ => vec![],
            },
            None => vec![],
        };
        let is_bare_type = self
            .builtins
            .get(&interner::intern("type"))
            .map(|t| t.is(&metacls))
            .unwrap_or(false);
        if is_bare_type && !kwargs.is_empty() {
            return Err(PyError::type_error(
                "type.__new__() takes no keyword arguments",
            ));
        }
        let metatype = if is_bare_type { None } else { Some(metacls) };
        self.default_build_class(name_str, bases_vec, namespace_dict, kwargs, metatype)
    }

    /// The plain (no custom metaclass) class-construction routine — this is
    /// the Rust equivalent of CPython's `type.__new__`: build the
    /// `PyObject::Type`, run C3 MRO linearization, apply `__set_name__` and
    /// `__init_subclass__`. Used directly for ordinary classes, and also
    /// exposed to Python code as `type.__new__` (see `type_new_builtin`
    /// below) so a custom metaclass's `__new__` can call
    /// `super().__new__(metacls, name, bases, namespace, **kwds)` and get
    /// this same construction — tagged with `metatype` so the result
    /// correctly reports which (customized) metaclass built it.
    pub(crate) fn default_build_class(
        &mut self,
        name_str: String,
        bases_vec: Vec<PyObjectRef>,
        mut namespace_dict: HashMap<String, PyObjectRef>,
        init_subclass_kwargs: Vec<(String, PyObjectRef)>,
        metatype: Option<PyObjectRef>,
    ) -> PyResult<PyObjectRef> {
        namespace_dict.remove("__annotation_tmp__");
        // Real CPython disallows subclassing `bool` outright (`TypeError:
        // type 'bool' is not an acceptable base type`) — unlike every other
        // migrated native type, `bool` is a real `PyObject::Type` (fixing
        // `type(True) is bool`) but deliberately NOT in
        // `is_recognized_native_base_name`, so it would otherwise fall
        // through to the generic `NATIVE_VALUE_CTOR_KEY`-based detection
        // arm just below and be silently treated as a valid native base
        // (constructing a nonsensical always-`False`-backed instance)
        // instead of raising. Checked by identity against the live `bool`
        // binding (not by name) so a shadowed/reassigned `bool` name
        // elsewhere doesn't false-positive.
        if let Some(bool_type) = self.builtins.get(&interner::intern("bool")) {
            for base in &bases_vec {
                if base.is(bool_type) {
                    return Err(PyError::type_error(
                        "type 'bool' is not an acceptable base type",
                    ));
                }
            }
        }

        // Detect `class Foo(list): ...` / `(dict)` / `(str)` / `(int)` —
        // either a direct native base, or inherited transitively through a
        // base that already carries the marker (propagated down so every
        // subclass's own dict has it, without needing to walk mro/bases
        // again at instantiation or dispatch time).
        for base in &bases_vec {
            let native_name = match &*base.borrow() {
                PyObject::BuiltinFunction { name, .. }
                    if crate::object::is_recognized_native_base_name(name) =>
                {
                    Some(name.clone())
                }
                // A native value type that's been migrated to a real
                // `PyObject::Type` (see `NATIVE_VALUE_CTOR_KEY`'s doc
                // comment — `int` as of this writing) is a second
                // recognized shape of "direct native base", alongside the
                // `BuiltinFunction` case above — `class MyInt(int): ...`
                // must keep working through this exact same
                // `NATIVE_BASE_MARKER`/native-backing machinery, unchanged.
                PyObject::Type { name, dict, .. }
                    if dict.contains_key_str(crate::object::NATIVE_VALUE_CTOR_KEY) =>
                {
                    Some(name.clone())
                }
                _ => crate::object::native_base_of_type(base),
            };
            if let Some(native_name) = native_name {
                namespace_dict.insert(
                    crate::object::NATIVE_BASE_MARKER.to_string(),
                    py_str(&native_name),
                );
                break;
            }
        }

        if let Some(mt) = &metatype {
            namespace_dict.insert(crate::object::METATYPE_KEY.to_string(), mt.clone());
        }

        // A class that defines `__eq__` but not `__hash__` gets
        // `__hash__ = None` (unhashable) — CPython's implicit rule
        // (class OnlyEquality: def __eq__(...): ... is unhashable).
        if namespace_dict.contains_key("__eq__") && !namespace_dict.contains_key("__hash__") {
            namespace_dict.insert("__hash__".to_string(), py_none());
        }

        let class = PyObjectRef::new(PyObject::Type {
            name: name_str,
            dict: Box::new(str_map_to_typedict(namespace_dict.clone())),
            bases: bases_vec.clone(),
            mro: vec![],
        });

        let mut mro = vec![class.clone()];
        // C3 linearization for proper method resolution
        let linearization = c3_linearize(&bases_vec)?;
        mro.extend(linearization);
        if let PyObject::Type { mro: mro_field, .. } = &mut *class.borrow_mut() {
            *mro_field = mro;
        }
        crate::object::register_class(&class);

        // Compute __abstractmethods__ for ABC support
        {
            use std::collections::HashSet;
            let mut abstracts: HashSet<String> = HashSet::new();
            for base in &bases_vec {
                if let Ok(am) = base.borrow().get_attribute("__abstractmethods__") {
                    match &*am.borrow() {
                        PyObject::FrozenSet(s) => {
                            for v in s.iter() { abstracts.insert(v.str()); }
                        }
                        PyObject::Set(s) => {
                            for v in s.iter() { abstracts.insert(v.str()); }
                        }
                        _ => {}
                    }
                }
            }
            let mut to_remove = Vec::new();
            for name in abstracts.iter() {
                if let Some(v) = namespace_dict.get(name) {
                    let is_abs = v.borrow().get_attribute("__isabstractmethod__").map(|b| b.truthy()).unwrap_or(false);
                    if !is_abs && !matches!(&*v.borrow(), PyObject::None) {
                        to_remove.push(name.clone());
                    }
                }
            }
            for n in to_remove { abstracts.remove(&n); }
            for (k, v) in &namespace_dict {
                if v.borrow().get_attribute("__isabstractmethod__").map(|b| b.truthy()).unwrap_or(false) {
                    abstracts.insert(k.clone());
                }
            }
            let mut set = crate::object::PySet::new();
            for n in abstracts.iter() { let _ = set.add(py_str(n)); }
            if let PyObject::Type { dict, .. } = &mut *class.borrow_mut() {
                dict.insert_str("__abstractmethods__", PyObjectRef::new(PyObject::FrozenSet(set)));
                if !dict.contains_key_str("_abc_registry") {
                    dict.insert_str("_abc_registry", PyObjectRef::new(PyObject::FrozenSet(crate::object::PySet::new())));
                }
            }
        }

        // __set_name__ protocol: for each descriptor in the class dict that has __set_name__, call it
        for (attr_name, value) in namespace_dict.iter() {
            // Get __set_name__ from the TYPE (not the instance) to avoid double-binding
            let typ = match &*value.borrow() {
                PyObject::Instance { typ, .. } => Some(typ.clone()),
                _ => None,
            };
            let has_set_name = if let Some(t) = &typ {
                t.borrow().get_attribute("__set_name__").is_ok()
            } else {
                false
            };
            if has_set_name {
                if let Some(t) = typ {
                    let set_name_method = t.borrow().get_attribute("__set_name__").unwrap();
                    // Call with explicit self=value, then owner=class, name=attr_name
                    if let Err(e) = self.call_function(
                        set_name_method,
                        vec![value.clone(), class.clone(), py_str(attr_name)],
                        vec![],
                    ) {
                        // Add __notes__ to the exception: CPython adds a note like
                        // "Error calling __set_name__ on 'Descriptor' instance 'attr' in 'Class'"
                        let class_name = if let PyObject::Type { name, .. } = &*class.borrow() {
                            name.clone()
                        } else {
                            "unknown".to_string()
                        };
                        let descr_name = if let PyObject::Type { name, .. } = &*t.borrow() {
                            name.clone()
                        } else {
                            value.borrow().type_name().to_string()
                        };
                        let note = format!(
                            "Error calling __set_name__ on '{}' instance '{}' in '{}'",
                            descr_name, attr_name, class_name
                        );
                        let exc_obj = match &e {
                            crate::object::PyError::Exception(_, obj) => obj.clone(),
                            _ => {
                                // Synthesize exception object from PyError
                                let typ = e.type_name_for_display();
                                let msg = e.message();
                                crate::object::PyObjectRef::new(crate::object::PyObject::Exception {
                                    typ: typ.clone(),
                                    args: vec![crate::object::py_str(&msg)],
                                    cause: None,
                                    suppress_context: false,
                                    context: None,
                                    traceback: None,
                                    extra: None,
                                })
                            }
                        };
                        {
                            let mut borrowed = exc_obj.borrow_mut();
                            if let crate::object::PyObject::Exception { extra, .. } = &mut *borrowed {
                                let map = extra.get_or_insert_with(|| std::collections::HashMap::new());
                                let old = map.get("__notes__").cloned().unwrap_or_else(|| crate::object::py_list(vec![]));
                                let mut items = if let crate::object::PyObject::List(v) = &*old.borrow() {
                                    v.clone()
                                } else {
                                    vec![]
                                };
                                items.push(crate::object::py_str(&note));
                                map.insert("__notes__".to_string(), crate::object::py_list(items));
                            } else if let crate::object::PyObject::Instance { dict, .. } = &mut *borrowed {
                                let notes = dict.get("__notes__").cloned().unwrap_or_else(|| crate::object::py_list(vec![]));
                                let mut items = if let crate::object::PyObject::List(v) = &*notes.borrow() { v.clone() } else { vec![] };
                                items.push(crate::object::py_str(&note));
                                dict.insert("__notes__".to_string(), crate::object::py_list(items));
                            }
                        }
                        // Re-raise with notes attached
                        match &e {
                            crate::object::PyError::Exception(typ, _) => {
                                return Err(crate::object::PyError::Exception(typ.clone(), exc_obj));
                            }
                            _ => {
                                let typ = e.type_name_for_display();
                                return Err(crate::object::PyError::Exception(typ, exc_obj));
                            }
                        }
                    }
                }
            }
        }

        // __init_subclass__ protocol: real CPython calls this EXACTLY ONCE
        // per class creation, via `super().__init_subclass__()` — which
        // walks the new class's own MRO (skipping the class itself) and
        // invokes the FIRST implementation found. This used to instead call
        // `get_attribute("__init_subclass__")` on every DIRECT base
        // independently, which — for any multiply-inherited class whose
        // several direct bases all resolve to the SAME shared ancestor
        // implementation (e.g. contextlib's `_GeneratorContextManager(
        // _GeneratorContextManagerBase, AbstractContextManager,
        // ContextDecorator)`, all sharing `object.__init_subclass__` — or,
        // more seriously, any two Django model mixins both resolving to
        // `AltersData.__init_subclass__`) called that ONE shared
        // implementation multiple times per class, redundantly at best and
        // — for an implementation with side effects, like Django's, which
        // lazily imports and re-walks `vars(cls)` — compounding into deep
        // reentrant recursion at worst (confirmed via a real repro: a
        // single `class MyModel(models.Model): pass` triggered 10+ nested
        // `AltersData.__init_subclass__` frames before failing).
        let self_mro = if let PyObject::Type { mro, .. } = &*class.borrow() {
            mro.clone()
        } else {
            vec![]
        };
        // Check each MRO entry's OWN direct dict (`get_str`), NOT the
        // recursive `get_attribute` (which re-walks THAT base's own MRO
        // from scratch and can resolve all the way down to `object`'s
        // shared no-op default on its own) — using `get_attribute` here
        // meant a multiply-inherited class whose FIRST base in MRO order
        // doesn't itself define `__init_subclass__` (e.g. a plain mixin
        // with no bases beyond implicit `object`) stopped at THAT base's
        // own inherited `object.__init_subclass__` default immediately,
        // never reaching a LATER base's real, meaningful override at all.
        // Real trigger: `class Combined(Mixin, unittest.TestCase): pass` —
        // `Mixin` (no explicit base) resolves `__init_subclass__` to
        // `object`'s default via its own separate MRO before `TestCase`'s
        // real override (which sets `_class_cleanups`, needed by
        // `TestCase.doClassCleanups`) is ever reached, silently skipping it
        // entirely. Checking each entry's OWN dict directly instead
        // correctly walks the single, already-flattened `self_mro` in
        // order — skipping bases with no direct definition — and still
        // calls the ultimate shared `object.__init_subclass__` default
        // exactly once if nothing else in the chain overrides it (this is
        // what the surrounding fix, described above, was for).
        let init_subclass = self_mro.iter().skip(1).find_map(|base| {
            if let PyObject::Type { dict, .. } = &*base.borrow() {
                dict.get_str("__init_subclass__").cloned()
            } else {
                None
            }
        });
        if let Some(init_subclass) = init_subclass {
            if std::env::var("RPY_DEBUG_INITSUBCLASS").is_ok() {
                let class_name = if let PyObject::Type { name, .. } = &*class.borrow() {
                    name.clone()
                } else {
                    "?".to_string()
                };
                eprintln!("INIT_SUBCLASS: class={}", class_name);
            }
            self.call_function(
                init_subclass,
                vec![class.clone()],
                init_subclass_kwargs.clone(),
            )?;
        }

        Ok(class)
    }

    /// Build a class via a custom metaclass (explicit `metaclass=` or one
    /// inherited from a base) — the general path real metaclasses (a
    /// user-defined class subclassing `type`, e.g. an enum's `EnumType`)
    /// need: look up `__new__` on the metaclass's own MRO and call it with
    /// the real CPython `__new__(metacls, name, bases, namespace, **kwds)`
    /// convention, falling back to the plain `default_build_class` (tagged
    /// with this metaclass) if the metaclass doesn't override `__new__`
    /// anywhere short of plain `type`. Also calls `__init__` on the
    /// metaclass afterward, if defined, mirroring normal instantiation.
    pub(crate) fn build_class_with_metaclass(
        &mut self,
        name_str: String,
        name_obj: PyObjectRef,
        bases_vec: Vec<PyObjectRef>,
        mut namespace_dict: HashMap<String, PyObjectRef>,
        order: Vec<String>,
        metaclass: PyObjectRef,
        init_subclass_kwargs: Vec<(String, PyObjectRef)>,
        prepared_namespace: Option<PyObjectRef>,
    ) -> PyResult<PyObjectRef> {
        namespace_dict.remove("__annotation_tmp__");
        // Ordered PyDict — class/metaclass namespace order is user-visible
        // (e.g. an enum's member definition order) and plain HashMap
        // iteration doesn't preserve it, so lay `order` down first. If the
        // metaclass's own `__prepare__` already produced a (now-populated)
        // namespace object — e.g. enum's `_EnumDict`, which tracked member
        // names via its own `__setitem__` as each entry was replayed into
        // it — use that object itself instead of building a fresh plain
        // dict, so extra attributes/state it accumulated (like
        // `_member_names`) survive into what the metaclass's `__new__`
        // receives.
        let namespace_py_dict = if let Some(prepared) = prepared_namespace {
            prepared
        } else {
            let mut pd = PyDict::new();
            for k in &order {
                if let Some(v) = namespace_dict.get(k) {
                    pd.set(py_str(k), v.clone())?;
                }
            }
            for (k, v) in &namespace_dict {
                if !order.contains(k) {
                    pd.set(py_str(k), v.clone())?;
                }
            }
            PyObjectRef::new(PyObject::Dict(Box::new(pd)))
        };
        let bases_tuple = PyObjectRef::imm(PyObject::Tuple(bases_vec.clone()));

        // `__new__` may be wrapped in StaticMethod (as `type.__new__` is,
        // and as a user metaclass's own `__new__` implicitly is too, since
        // `__new__` is always an implicit staticmethod in real Python) —
        // unwrap before calling, same as Type's own get_attribute does for
        // plain class-attribute access.
        let new_fn = crate::object::lookup_dunder_via_mro(&metaclass, "__new__").map(|v| {
            let unwrapped = if let PyObject::StaticMethod { func } = &*v.borrow() {
                Some(func.clone())
            } else {
                None
            };
            unwrapped.unwrap_or(v)
        });

        let cls = if let Some(new_fn) = new_fn {
            if std::env::var("RPY_DEBUG_METACLASS").is_ok() {
                eprintln!(
                    "build_class_with_metaclass: name={} metaclass={} new_fn={}",
                    name_str,
                    metaclass.repr(),
                    new_fn.repr()
                );
            }
            self.call_function(
                new_fn,
                vec![
                    metaclass.clone(),
                    name_obj.clone(),
                    bases_tuple.clone(),
                    namespace_py_dict.clone(),
                ],
                init_subclass_kwargs.clone(),
            )?
        } else {
            // No __new__ anywhere in the metaclass's own mro (shouldn't
            // normally happen once `type` is registered with one) — fall
            // back to plain construction, still tagged with this metaclass.
            self.default_build_class(
                name_str,
                bases_vec,
                namespace_dict,
                init_subclass_kwargs.clone(),
                Some(metaclass.clone()),
            )?
        };

        if let Some(init_fn) = crate::object::lookup_dunder_via_mro(&metaclass, "__init__") {
            let unwrapped = if let PyObject::StaticMethod { func } = &*init_fn.borrow() {
                Some(func.clone())
            } else {
                None
            };
            let init_fn = unwrapped.unwrap_or(init_fn);
            self.call_function(
                init_fn,
                vec![cls.clone(), name_obj, bases_tuple, namespace_py_dict],
                init_subclass_kwargs,
            )?;
        }

        // ABC support: same as default_build_class — trigger when any MRO
        // entry is abc.ABC OR when the metaclass is abc.ABCMeta.
        {
            let abc_mod = self.modules.get("abc");
            let (abc_type, update_fn) = match &abc_mod {
                Some(m) => (
                    m.borrow().get_attribute("ABC").ok(),
                    m.borrow().get_attribute("update_abstractmethods").ok(),
                ),
                None => (None, None),
            };
            let has_abc = abc_type.map(|abc_t| {
                let cls_mro = if let PyObject::Type { mro, .. } = &*cls.borrow() {
                    mro.clone()
                } else { vec![] };
                cls_mro.iter().any(|base| base.is(&abc_t))
            }).unwrap_or(false);
            // Also trigger if the metaclass is ABCMeta itself
            let is_abc_meta = abc_mod.as_ref().map(|_| {
                let name = metaclass.borrow().type_name();
                name == "ABCMeta" || name == "builtin_function_or_method"
                    && metaclass.borrow().repr().contains("ABCMeta")
            }).unwrap_or(false);
            if has_abc || is_abc_meta {
                if let Some(update) = update_fn {
                    let _ = self.call_function(update, vec![cls.clone()], vec![]);
                }
            }
        }

        Ok(cls)
    }

    /// Call __next__ on a user-class iterator. Used by FOR_ITER for Instance types.
    pub(crate) fn for_iter_next(
        &mut self,
        iter_val: PyObjectRef,
        jump_offset: u32,
    ) -> PyResult<Option<PyObjectRef>> {
        use crate::object::ObjectAccess;
        // If this generator's body contains `yield from`, remember the active
        // sub-iterator so an incoming .throw() delegates to it (CPython
        // semantics) instead of injecting into the outer frame.
        let has_yf = self
            .frames
            .last()
            .map(|f| f.code.flags & 0x0200 != 0)
            .unwrap_or(false);
        if has_yf {
            self.frames.last_mut().unwrap().yield_from_iter = Some(iter_val.clone());
        }
        match crate::object::builtin_next(&[iter_val.clone()]) {
            Ok(val) => {
                self.frames.last_mut().unwrap().push(iter_val);
                self.frames.last_mut().unwrap().push(val);
                Ok(None)
            }
            Err(e) if crate::object::is_stop_iteration_error(&e) => {
                if has_yf {
                    self.frames.last_mut().unwrap().yield_from_iter = None;
                }
                self.frames.last_mut().unwrap().ip = jump_offset as usize;
                Ok(None)
            }
            Err(e) => Err(e),
        }
    }
}
