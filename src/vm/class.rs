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
        let mut bases_vec = match &*args[2].borrow() {
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
        // Real CPython's `type.__new__` always adds `object` as the
        // implicit base when no explicit bases are given (empty tuple
        // or `None`). Without this, the class's MRO is missing `object`
        // and `__init_subclass__` never fires.
        if bases_vec.is_empty() {
            if let Some(object_type) = self.builtins.get(&interner::intern("object")) {
                bases_vec.push(object_type.clone());
            }
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
        // Real `type.__new__` consumes `__qualname__` from the namespace
        // (it becomes the type's qualname slot, NOT a `__dict__` entry) —
        // real CPython class `__dict__`s never contain it (verified: plain
        // classes and typing.Protocol classes alike). The class body always
        // seeds the implicit `__qualname__ = <bare class name>` first, so
        // strip exactly that seed; an EXPLICIT `__qualname__ = "X"` in the
        // body (which real CPython honors as the type's qualname) is left
        // in place, preserving today's behavior for it. Without this,
        // typing's `_get_protocol_attrs` collects the spurious entry (a
        // plain string, non-callable) and every `isinstance(x,
        // SupportsInt)`-style check raises `TypeError: Protocols with
        // non-method members...` (test_fractions).
        if let Some(qn) = namespace_dict.get("__qualname__") {
            if qn.str() == name_str {
                namespace_dict.remove("__qualname__");
            }
        }
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

        // Generic version of the same restriction, for native types marked
        // via `NO_SUBCLASS_KEY` (currently `contextvars.ContextVar`/
        // `Context`/`Token`) rather than another one-off identity check —
        // see that constant's own doc comment.
        for base in &bases_vec {
            if let Some(name) = crate::object::no_subclass_type_name(base) {
                return Err(PyError::type_error(format!(
                    "type '{}' is not an acceptable base type",
                    name
                )));
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
            name: name_str.clone(),
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
        // Custom metaclass `mro()` hook (gh-127773): real CPython calls
        // `metaclass.mro(new_class)` ONCE at class creation (only when the
        // metaclass is customized, never for bare `type`) and adopts the
        // result as the effective MRO — later attribute access uses the
        // cached MRO without re-invoking it. Only a Python-level `def`
        // override counts: the lookup is a raw MRO-dict walk (never
        // `get_attribute`, whose synthesized builtin `mro` fallback
        // ignores its argument and would install a wrong MRO).
        if metatype.is_some() {
            let custom_mro_fn: Option<PyObjectRef> = (|| {
                // Walk the metaclass's own MRO for a Python-level `mro`.
                let mc = metatype.as_ref()?;
                let mc_borrow = mc.borrow();
                let PyObject::Type { mro: mc_mro, .. } = &*mc_borrow else {
                    return None;
                };
                for base in mc_mro {
                    if let PyObject::Type { dict: bdict, .. } = &*base.borrow() {
                        if let Some(f) = bdict.get_str("mro") {
                            if matches!(&*f.borrow(), PyObject::Function(_)) {
                                return Some(f.clone());
                            }
                        }
                    }
                }
                None
            })();
            if let Some(mro_fn) = custom_mro_fn {
                let result = self.call_function(mro_fn, vec![class.clone()], vec![])?;
                let items: Vec<PyObjectRef> = match &*result.borrow() {
                    PyObject::Tuple(t) => t.clone(),
                    PyObject::List(l) => l.clone(),
                    _ => {
                        return Err(PyError::type_error("mro() must return a tuple"));
                    }
                };
                if items.is_empty() {
                    return Err(PyError::type_error("type MRO must not be empty"));
                }
                let first_is_cls = items
                    .first()
                    .map(|f| f.is(&class))
                    .unwrap_or(false);
                if !first_is_cls {
                    return Err(PyError::type_error(
                        "mro() must return the class itself first",
                    ));
                }
                if let PyObject::Type { mro: mro_field, .. } = &mut *class.borrow_mut() {
                    *mro_field = items;
                }
            }
        }

        // PEP 560: inject __orig_bases__ into class dict BEFORE __init_subclass__
        // so typing.py's Generic.__init_subclass__ can inspect the original bases.
        if let Some(orig_bases) = namespace_dict.get("__orig_bases__").cloned() {
            if let PyObject::Type { dict, .. } = &mut *class.borrow_mut() {
                dict.insert_str("__orig_bases__", orig_bases);
            }
        }

        // NOTE: this used to unconditionally compute and set
        // `__abstractmethods__` (and a bogus empty `_abc_registry`
        // `FrozenSet`) on EVERY class built here, regardless of whether it
        // has anything to do with `abc.ABC`/`ABCMeta` at all — a workaround
        // from when this codebase's `abc.ABCMeta` was a native stub whose
        // own class-creation logic never computed this itself. Real CPython
        // does NOT do this: `hasattr(cls, '__abstractmethods__')` is
        // `False` for a plain `class Foo: pass` (confirmed against a local
        // CPython 3.14 install) — only a class actually built through
        // `ABCMeta.__new__` gets the attribute at all. Since `abc.ABCMeta`/
        // `_py_abc.ABCMeta` are now real Python metaclasses
        // (`Lib/abc.py`/`Lib/_py_abc.py`) whose own `__new__` computes
        // `__abstractmethods__` itself (via `build_class_with_metaclass`'s
        // generic custom-metaclass dispatch — this function only runs for
        // it a SECOND time via the `super().__new__()` chain, harmlessly
        // superseded by ABCMeta's own subsequent assignment), this
        // unconditional block is now not just redundant but actively wrong
        // for ordinary classes: `abc.update_abstractmethods(cls)` on a
        // plain class checks `hasattr(cls, '__abstractmethods__')` and
        // no-ops if absent (matching real CPython) — with this attribute
        // wrongly pre-seeded to an empty frozenset here, that check instead
        // proceeded to WRONGLY compute and attach real `__abstractmethods__`
        // to a plain class after the fact (`test_abc.py`'s
        // `test_update_non_abc`: assigning an `@abstractmethod`-decorated
        // function onto a plain class and calling `update_abstractmethods`
        // on it should be a no-op, not retroactively turn the class
        // abstract).
        //
        // It is, however, still needed in one specific, narrower shape: this
        // codebase's NATIVE `collections.abc`/`_collections_abc` module
        // (`src/modules/misc/collections.rs`) builds `Mapping`/
        // `MutableMapping`/etc. as bare `PyObject::Type`s with a hardcoded
        // `__abstractmethods__`, NOT through real `ABCMeta` (unlike real
        // CPython, where `_collections_abc.py`'s classes genuinely are
        // `metaclass=ABCMeta`, so a subclass inherits that metaclass and
        // gets its `__abstractmethods__` recomputed by `ABCMeta.__new__`
        // automatically) — so a plain-metaclass subclass of one of these
        // (e.g. `weakref.WeakKeyDictionary(_collections_abc.MutableMapping)`,
        // which implements every method `MutableMapping` marks abstract)
        // needs SOMETHING to recompute its own `__abstractmethods__` down to
        // empty, or the unmodified, inherited-via-mro-walk non-empty set
        // from `MutableMapping` wrongly blocks instantiation entirely.
        // Restoring the recomputation is safe as long as it's gated on "at
        // least one base ALREADY carries `__abstractmethods__`" (checked
        // before computing anything) — real CPython's plain `type.__new__`
        // never introduces the attribute from nothing (an ordinary class
        // marking its own methods `@abstractmethod` with no ABC ancestry at
        // all stays fully instantiable, matching the `test_update_non_abc`
        // fix above), it only ever narrows what's already there.
        {
            use std::collections::HashSet;
            let any_base_has_abstracts = bases_vec
                .iter()
                .any(|base| base.borrow().get_attribute("__abstractmethods__").is_ok());
            if any_base_has_abstracts {
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
        // Bases tuple for the metaclass protocol (`__new__`/`__init__`/
        // direct metaclass calls): the resolved bases, but EMPTY (not
        // `(object,)`) when no bases were written — real CPython passes
        // `()` there while still defaulting the created class to
        // `(object,)`. Computed by the caller, which knows original
        // emptiness (already normalized away in `bases_vec`).
        protocol_bases: PyObjectRef,
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
            // The prepared namespace (from the metaclass's own
            // `__prepare__`, including the default `type.__prepare__`
            // every custom metaclass inherits) is the SAME object the
            // class body's `STORE_NAME`s replayed into live, but
            // `__orig_bases__` is computed by Rust code (see caller) AFTER
            // that body already finished executing, and was only ever
            // inserted into the separate `namespace_dict` HashMap — never
            // into this actual `prepared` dict object. Any class with an
            // EXPLICIT metaclass (which is exactly when this branch is
            // taken) therefore silently lost `__orig_bases__` even though
            // `namespace_dict` had it, breaking `typing.Generic`'s
            // `__init_subclass__`/`__class_getitem__` for any custom-
            // metaclass class combined with a `__mro_entries__`-resolved
            // base (real trigger: `class Foo(TypedDict, Generic[T])`,
            // whose `_TypedDictMeta.__new__` is a custom metaclass).
            if let Some(orig_bases) = namespace_dict.get("__orig_bases__") {
                if let PyObject::Dict(d) = &mut *prepared.borrow_mut() {
                    let _ = d.set(py_str("__orig_bases__"), orig_bases.clone());
                }
            }
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
        let bases_tuple = protocol_bases;

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

        // A non-type metaclass (plain function): real CPython just calls
        // `meta(name, bases, namespace, **kwds)` and uses whatever it
        // returns as the class object (often the namespace itself) — no
        // `__new__`/`__init__` protocol at all.
        if !matches!(&*metaclass.borrow(), PyObject::Type { .. }) {
            return self.call_function(
                metaclass.clone(),
                vec![
                    name_obj.clone(),
                    bases_tuple.clone(),
                    namespace_py_dict.clone(),
                ],
                init_subclass_kwargs.clone(),
            );
        }

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
                // `iter_val` was only *peeked* (not popped) off the stack by
                // the FOR_ITER caller, so pop that peeked copy before
                // re-pushing it — otherwise the iterator ends up duplicated
                // on the stack, one copy of the pair too many. That leftover
                // entry desyncs the frame's stack layout for the rest of the
                // loop: once this (inner) loop exits and its own
                // iterator+POP_TOP cleanup runs, the extra garbage entry
                // remains behind, corrupting an enclosing (outer) loop's own
                // stack — e.g. `for i in range(3): \n for x in
                // custom_iter_with___next__(): pass` silently terminated the
                // OUTER loop after just one iteration.
                self.frames.last_mut().unwrap().pop()?;
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
