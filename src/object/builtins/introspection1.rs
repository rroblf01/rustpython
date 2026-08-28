// Split out of the former monolithic object/builtins.rs — this file holds
// introspection and type-checking builtins (`isinstance`, `issubclass`,
// `dir`, `vars`, `hash`, `id`, `slice`, `open`, `help`, etc.) and the
// call-dispatch helpers.
use super::*;

/// Does `typ`'s mro/bases include a builtin exception class (`Exception`,
/// `OSError`, ...)? Those are `PyObject::BuiltinFunction` (the exception's
/// own constructor), never a real `PyObject::Type` — invisible to
/// `lookup_dunder_via_mro`'s dict-based walk, so a plain `class MyError
/// (Exception): pass` (no custom `__str__`/`__repr__` override) fell
/// through to the fully-generic `<MyError object>` instead of real
/// `BaseException.__str__`/`__repr__`'s args-based formatting (`str(exc)`
/// for a single-arg exception should be that arg's `str()`, not a useless
/// placeholder — this broke essentially every custom exception hierarchy's
/// error messages).
/// Read a class's OWN `_abc_registry` (set by `.register()`, the generic
/// `PyObject::Type` fallback method — see its own doc comment) — NEVER via
/// `get_attribute`/`lookup_dunder_via_mro`-style MRO walking. A virtual
/// registration against one ABC (`Complex.register(complex)`) must NOT be
/// visible when checking a MORE SPECIFIC descendant ABC (`Integral`) just
/// because `Integral` inherits from `Complex` — registration doesn't flow
/// "downward" in specificity that way. (`.register()` itself has the exact
/// same "must not read via MRO" requirement, for a different reason — see
/// its own inline comment.)
pub(crate) fn own_abc_registry(typ: &PyObjectRef) -> Vec<PyObjectRef> {
    if let PyObject::Type { dict, .. } = &*typ.borrow() {
        dict.get_str("_abc_registry")
            .and_then(|r| {
                if let PyObject::FrozenSet(items) = &*r.borrow() {
                    Some(items.to_vec())
                } else {
                    None
                }
            })
            .unwrap_or_default()
    } else {
        Vec::new()
    }
}


/// Is anything matching `matcher` registered against `base` ITSELF, or
/// against any REAL (regular inheritance, non-virtual) subclass of `base`?
/// Needed because registration should propagate to less-specific ABCs in
/// the same real hierarchy: `numbers.py`'s `Integral.register(int)` must
/// also make `issubclass(int, Real)`/`issubclass(int, Complex)` true,
/// since `Integral` is a real subclass of `Real`/`Complex` (NOT because
/// `int` itself was registered against them directly — it wasn't). Walks
/// `direct_subclasses_of` recursively; cheap in practice since real ABC
/// hierarchies (`numbers`, `collections.abc`) are shallow.
pub(crate) fn abc_registry_matches_in_subtree(
    base: &PyObjectRef,
    matcher: &dyn Fn(&PyObjectRef) -> bool,
) -> bool {
    if own_abc_registry(base).iter().any(|r| matcher(r)) {
        return true;
    }
    direct_subclasses_of(base)
        .iter()
        .any(|sub| abc_registry_matches_in_subtree(sub, matcher))
}


pub(crate) fn is_exception_type(typ: &PyObjectRef) -> bool {
    if let PyObject::Type { mro, bases, .. } = &*typ.borrow() {
        let entries = if mro.is_empty() { bases } else { mro };
        entries.iter().any(|b| {
            if let PyObject::BuiltinFunction { name, .. } = &*b.borrow() {
                is_builtin_exception_class_name(name)
            } else {
                false
            }
        })
    } else {
        false
    }
}


/// Real `BaseException.__str__`: no args -> `""`, one arg -> that arg's
/// `str()`, multiple args -> `str()` of the whole args tuple.
pub(crate) fn exception_instance_str(instance: &PyObjectRef) -> String {
    let args = if let PyObject::Instance { dict, .. } = &*instance.borrow() {
        dict.get("args").cloned()
    } else {
        None
    };
    match args.map(|a| a.borrow().clone()) {
        Some(PyObject::Tuple(items)) if items.len() == 1 => items[0].str(),
        Some(PyObject::Tuple(items)) if !items.is_empty() => py_tuple(items).str(),
        _ => String::new(),
    }
}


/// Real `BaseException.__repr__`: `ClassName(repr(arg1), repr(arg2), ...)`.
pub(crate) fn exception_instance_repr(instance: &PyObjectRef, class_name: &str) -> String {
    let args = if let PyObject::Instance { dict, .. } = &*instance.borrow() {
        dict.get("args").cloned()
    } else {
        None
    };
    let args_str = match args.map(|a| a.borrow().clone()) {
        Some(PyObject::Tuple(items)) => items
            .iter()
            .map(|a| a.repr())
            .collect::<Vec<_>>()
            .join(", "),
        _ => String::new(),
    };
    format!("{}({})", class_name, args_str)
}


pub fn builtin_object(_args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    // Create a new bare object instance
    let object_type = PyObjectRef::new(PyObject::Type {
        name: "object".to_string(),
        dict: Box::new(TypeDict::default()),
        bases: vec![],
        mro: vec![],
    });
    Ok(PyObjectRef::new(PyObject::Instance {
        typ: object_type,
        dict: AttrMap::new(),
    }))
}


pub fn builtin_hash(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 1 {
        return Err(PyError::type_error("hash() takes exactly one argument"));
    }
    // Delegate entirely to `PyObjectRef::hash()` — the SAME method
    // `PyDict`/`PySet` call internally to hash a key. This used to
    // reimplement a SEPARATE algorithm per type here (FNV-1a for
    // str/bytes/bytearray/tuple, vs. the byte-multiplier/char-multiplier
    // algorithms `PyObjectRef::hash()`/`PyObject::hash()` actually use for
    // dict/set storage) — meaning `hash(x)` as seen by Python code could
    // disagree with the hash ACTUALLY used when `x` is placed in a dict/set,
    // a correctness invariant `hash()` exists specifically to uphold.
    // Confirmed via `hash("hello")` returning a completely different value
    // depending on whether "hello" happened to be represented as an inline
    // `SmallStr` or a boxed `PyObject::Str` — both must, and now do, agree.
    // This also fixes a second bug: the old `_` catch-all silently hashed
    // genuinely unhashable types (`list`, `dict`, `set`) by pointer instead
    // of raising `TypeError: unhashable type: '...'` like real Python.
    Ok(py_int(args[0].hash()? as i64))
}


pub fn builtin_slice(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    match args.len() {
        1 => {
            let stop = args[0].clone();
            let none = py_none();
            Ok(PyObjectRef::imm(PyObject::Slice {
                start: none.clone(),
                stop,
                step: none,
            }))
        }
        2 => {
            let start = args[0].clone();
            let stop = args[1].clone();
            let none = py_none();
            Ok(PyObjectRef::imm(PyObject::Slice {
                start,
                stop,
                step: none,
            }))
        }
        3 => Ok(PyObjectRef::imm(PyObject::Slice {
            start: args[0].clone(),
            stop: args[1].clone(),
            step: args[2].clone(),
        })),
        _ => Err(PyError::type_error("slice() takes at most 3 arguments")),
    }
}


pub fn builtin_dir(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return crate::object::with_vm_mut(|vm| {
            let frame = vm
                .frames
                .last()
                .ok_or_else(|| crate::object::PyError::runtime_error("no frame"))?;
            let mut names: Vec<PyObjectRef> = Vec::new();
            let mut seen = std::collections::HashSet::new();
            // Locals via InternedMap (used for some scopes)
            for (k, _) in frame.locals.iter() {
                let s = crate::interner::lookup_str(k);
                if seen.insert(s.to_string()) {
                    names.push(py_str(s));
                }
            }
            // Fast locals via varnames
            for (idx, var_id) in frame.code.varnames.iter().enumerate() {
                if frame
                    .fast_locals
                    .get(idx)
                    .and_then(|v| v.as_ref())
                    .is_some()
                {
                    let s = crate::interner::lookup_str(*var_id);
                    if seen.insert(s.to_string()) {
                        names.push(py_str(s));
                    }
                }
            }
            // For module frames, locals/varnames may be empty, but globals holds the module dict
            if names.is_empty() {
                for (k, _) in frame.globals.borrow().iter() {
                    let s = crate::interner::lookup_str(*k);
                    if seen.insert(s.to_string()) {
                        names.push(py_str(s));
                    }
                }
            }
            names.sort_by(|a, b| a.str().cmp(&b.str()));
            Ok(py_list(names))
        })?;
    }
    let obj = args[0].borrow();
    let mut names = Vec::new();
    match &*obj {
        PyObject::Instance { dict, typ } => {
            for key in dict.keys() {
                names.push(py_str(key));
            }
            // Instance dir() must also surface the class's own and inherited
            // members (CPython: dir(x) == sorted(set(vars(x)) | union of
            // vars over type(x).__mro__)). Without this, `dir(obj)` on any
            // user class omitted every method -- breaking patterns like
            // configparser's ConverterMapping, which discovers getters via
            // dir(parser).
            if let PyObject::Type {
                dict: tdict,
                mro,
                ..
            } = &*typ.borrow()
            {
                let mut seen = std::collections::HashSet::new();
                for key in tdict.keys() {
                    let s = interner::lookup_str(*key);
                    if s.starts_with("__native") {
                        continue;
                    }
                    if seen.insert(*key) {
                        names.push(py_str(s));
                    }
                }
                for base in mro {
                    if let PyObject::Type {
                        dict: base_dict, ..
                    } = &*base.borrow()
                    {
                        for key in base_dict.keys() {
                            let s = interner::lookup_str(*key);
                            if s.starts_with("__native") {
                                continue;
                            }
                            if seen.insert(*key) {
                                names.push(py_str(s));
                            }
                        }
                    }
                }
            }
        }
        PyObject::Module { dict, .. } => {
            for key in dict.keys() {
                names.push(py_str(interner::lookup_str(*key)));
            }
        }
        PyObject::Type { dict, mro, .. } => {
            // `dir(SomeClass)` must include every name reachable via the
            // class's OWN dict AND every ancestor's (`mro`) — real
            // CPython's `dir()` on a class is `sorted(set().union(*(vars(c)
            // for c in cls.__mro__)))`. This previously only read the
            // class's OWN dict, so `dir()` on ANY class with inherited
            // members (i.e. virtually every class with a base other than
            // bare `object`) silently omitted every name defined on a
            // parent — confirmed via `class Combined(Mixin, unittest.
            // TestCase): pass`, where `dir(Combined)` omitted `Mixin`'s own
            // `test_*` methods even though `hasattr`/`getattr` found them
            // fine (a real, general attribute-LOOKUP-vs-`dir()`-enumeration
            // gap this was hiding) — `unittest`'s own `TestLoader.
            // getTestCaseNames` uses exactly this `dir()` call to discover
            // test methods, so this silently dropped every test defined on
            // a mixin base for any multiple-inheritance test class.
            let mut seen = std::collections::HashSet::new();
            for key in dict.keys() {
                let s = interner::lookup_str(*key);
                if s.starts_with("__native") {
                    continue;
                }
                if seen.insert(*key) {
                    names.push(py_str(s));
                }
            }
            for base in mro {
                if let PyObject::Type {
                    dict: base_dict, ..
                } = &*base.borrow()
                {
                    for key in base_dict.keys() {
                        let s = interner::lookup_str(*key);
                        if s.starts_with("__native") {
                            continue;
                        }
                        if seen.insert(*key) {
                            names.push(py_str(s));
                        }
                    }
                }
            }
        }
        _ => {}
    }
    // Add basic attributes for all types EXCEPT modules — real
    // `dir(some_module)` is just `sorted(vars(some_module).keys())`, with
    // no implicit `__class__`/`__dir__` added (those come from a type's
    // MRO walk, which modules don't participate in the same way instances/
    // classes do). Confirmed via `test_pkg.py`, which compares `dir()` on
    // freshly-imported packages against an exact expected list that does
    // NOT include either.
    if !matches!(&*obj, PyObject::Module { .. }) {
        names.push(py_str("__class__"));
        names.push(py_str("__dir__"));
    }
    names.sort_by(|a, b| {
        let a = a.borrow();
        let b = b.borrow();
        if let (PyObject::Str(a), PyObject::Str(b)) = (&*a, &*b) {
            a.cmp(b)
        } else {
            std::cmp::Ordering::Equal
        }
    });
    Ok(py_list(names))
}


pub fn builtin_globals(_args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    with_vm_mut(|vm| {
        let frame = vm
            .frames
            .last()
            .ok_or_else(|| PyError::runtime_error("no frame"))?;
        // Return a LIVE view of the frame's globals so mutations
        // (`globals()['len'] = f`, test_dynamic::test_globals_shadow_builtins)
        // are visible to `LOAD_GLOBAL`, which reads the same backing map.
        Ok(PyObjectRef::new(PyObject::Globals(frame.globals.clone())))
    })?
}


pub fn builtin_locals(_args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    with_vm_mut(|vm| {
        let frame = vm
            .frames
            .last()
            .ok_or_else(|| PyError::runtime_error("no frame"))?;
        let mut d = crate::object::PyDict::new();
        for (k, v) in frame.locals.iter() {
            let name = crate::interner::lookup(k);
            d.set(py_str(&name), v.clone())?;
        }
        Ok(PyObjectRef::new(PyObject::Dict(Box::new(d))))
    })?
}




/// Invoke `func` in a fresh disposable VM, supporting KEYWORD arguments —
/// unlike `call_bound_method` (which only forwards positionals). Needed by
/// `atexit._run_exitfuncs` (`register(func, 3, key='value')` callbacks), and
/// generally useful for running a user `Function` from native code with the
/// full calling convention without re-entering the live VM's execute loop
/// (which is what `vm.call_function` does from inside a builtin, and which
/// misbehaves for user Functions).
pub fn call_function_disposable(
    func: &PyObjectRef,
    args: Vec<PyObjectRef>,
    keywords: Vec<(String, PyObjectRef)>,
) -> PyResult<PyObjectRef> {
    match &*func.borrow() {
        PyObject::BuiltinFunction { func: f, .. } => f(&args),
        PyObject::Closure(c) => c(&args),
        PyObject::BuiltinMethod {
            func: f, self_obj, ..
        } => {
            let mut all = vec![self_obj.clone()];
            all.extend(args);
            f(&all)
        }
        PyObject::BoundMethod { func, self_obj } => {
            // A bound method (user-defined `self.method`) reached through the
            // disposable path — prepend its bound self, then dispatch the
            // underlying callable (real trigger: atexit's `_run_exitfuncs`
            // reporting through a `sys.unraisablehook` that is a bound method,
            // as `test.support.catch_unraisable_exception` installs).
            let mut all = vec![self_obj.clone()];
            all.extend(args);
            let _guard = crate::object::NativeDispatchRecursionGuard::enter()?;
            let mut vm = crate::vm::VirtualMachine::new();
            vm.call_function(func.clone(), all, keywords)
        }
        PyObject::Function(_) => {
            let _guard = crate::object::NativeDispatchRecursionGuard::enter()?;
            let mut vm = crate::vm::VirtualMachine::new();
            vm.call_function(func.clone(), args, keywords)
        }
        PyObject::Type { .. } => {
            // Calling a class (user class / namedtuple factory result /
            // builtin type) from a native closure — route through a fresh VM
            // so instance creation runs the normal __new__/__init__ path.
            let _guard = crate::object::NativeDispatchRecursionGuard::enter()?;
            let mut vm = crate::vm::VirtualMachine::new();
            vm.call_function(func.clone(), args, keywords)
        }
        _ => Err(PyError::type_error(format!(
            "'{}' object is not callable",
            func.borrow().type_name()
        ))),
    }
}


pub fn call_bound_method(
    func: PyObjectRef,
    self_obj: PyObjectRef,
    args: Vec<PyObjectRef>,
) -> PyResult<PyObjectRef> {
    match &*func.borrow() {
        PyObject::BuiltinMethod {
            func: f,
            self_obj: s,
            ..
        } => {
            let mut all_args = vec![s.clone()];
            all_args.push(self_obj);
            all_args.extend(args);
            f(&all_args)
        }
        PyObject::BuiltinFunction { func: f, .. } => {
            let mut all_args = vec![self_obj];
            all_args.extend(args);
            f(&all_args)
        }
        PyObject::Closure(func) => {
            let mut all_args = vec![self_obj];
            all_args.extend(args);
            func(&all_args)
        }
        PyObject::Function(ref f) => {
            // See `NativeDispatchRecursionGuard`'s own doc comment (`core.rs`)
            // — without this, recursion flowing through this disposable-VM
            // dispatch path (e.g. `A.__call__ = A(); A()()`) overflows the
            // real native stack instead of raising a catchable
            // `RecursionError`, since each nested call resets its own fresh
            // VM's frame counter to zero.
            let _guard = crate::object::NativeDispatchRecursionGuard::enter()?;
            let code = &f.code;
            let g = &f.globals;
            let defaults = &f.defaults;
            let fname = &f.code.name;
            let closure = &f.closure;
            if std::env::var("RPY_DEBUG_IMPORT").is_ok() {
                eprintln!(
                    "CALL_BOUND_METHOD (disposable VM): fname={} code_name={} filename={}",
                    fname, code.name, code.filename
                );
            }
            if std::env::var("RPY_DEBUG_CBM").is_ok() {
                eprintln!(
                    "call_bound_method: fname={} varnames={:?} args.len()={} arg_count={}",
                    fname,
                    code.varnames,
                    args.len(),
                    code.arg_count
                );
            }
            // The disposable VM is constructed BEFORE the frame (and the
            // frame borrows ITS `builtins` map, not a separately-built one)
            // deliberately: `vm.rs`'s `call_function` special-cases `type(x)`
            // (and a few other builtins) by POINTER IDENTITY against
            // `self.builtins.get("type")` — if the frame's own `builtins` map
            // were built via a second, independent `create_builtins()` call
            // (as this used to do), it would contain a structurally-identical
            // but NOT pointer-identical "type" object, so that identity check
            // silently fails and `type(x)` falls through to being treated as
            // an ordinary call to the `type` class (constructing a bogus
            // instance-of-`type`) instead of returning `x`'s real type.
            // Confirmed general via a Django-free repro: `type(self).__name__`
            // inside a function invoked through `call_bound_method` (e.g. any
            // user-defined `__repr__` reached via the `repr()` builtin)
            // raised `AttributeError: 'type' object has no attribute
            // '__name__'` — `type(self)` had silently returned a fresh
            // `type`-instance instead of the real class object.
            let mut vm = crate::vm::VirtualMachine::new();
            let mut frame = crate::vm::Frame::new(
                code.clone(),
                g.clone(),
                std::rc::Rc::clone(&vm.builtins),
                None,
            );
            // Without this, ANY closure-capturing function invoked via
            // call_bound_method (repr()/str()/hash()/comparisons/other
            // native builtins that call a user-defined dunder this way,
            // instead of through the normal CALL opcode's own frame setup
            // in vm.rs, which already does set this) silently lost every
            // free variable — "variable 'x' not found" the moment the
            // function's body referenced one. Confirmed general via a
            // Django-free repro: a `dataclass`-generated `__repr__` closing
            // over its own field-name list worked fine called directly
            // (`obj.__repr__()`) but raised NameError through `repr(obj)`.
            frame.closure = Box::new(closure.clone());
            let code = code.clone();
            let defaults = defaults.clone();
            // Set self at index 0
            if !code.varnames.is_empty() {
                frame.fast_locals[0] = Some(self_obj.clone());
                frame.insert_local(crate::interner::lookup_str(code.varnames[0]), self_obj);
            }
            let npos = args.len();
            let named_params = if code.vararg_name.is_some() || code.kwarg_name.is_some() {
                code.varnames
                    .iter()
                    .position(|n| {
                        code.vararg_name.as_ref().map(|b| b.as_str())
                            == Some(crate::interner::lookup_str(*n))
                            || code.kwarg_name.as_ref().map(|b| b.as_str())
                                == Some(crate::interner::lookup_str(*n))
                    })
                    .unwrap_or(code.varnames.len())
            } else {
                code.varnames.len()
            };
            for i in 0..npos.min(named_params.saturating_sub(1)) {
                let idx = i + 1;
                if idx < code.varnames.len() {
                    frame.fast_locals[idx] = Some(args[i].clone());
                    frame.insert_local(
                        crate::interner::lookup_str(code.varnames[idx]),
                        args[i].clone(),
                    );
                }
            }
            if let Some(vararg_name) = &code.vararg_name {
                let mut extra = Vec::new();
                for i in (named_params.saturating_sub(1))..npos {
                    extra.push(args[i].clone());
                }
                let vararg_val = py_tuple(extra);
                // Must ALSO land in `fast_locals` — `LOAD_FAST` reads that,
                // not the `insert_local` name dict. Missing this meant any
                // `*args`-taking function/method invoked through THIS
                // disposable-VM path (constructing an instance whose class
                // was invoked via `map()`/`filter()`/etc., e.g. `map(TestCase,
                // testMethodNames)` calling each `TestCase.__init__(self,
                // *args, **kwargs)`) raised "local variable 'args' referenced
                // before assignment" the instant the function body read its
                // own vararg tuple — real trigger: CPython's own
                // `test_descr.py`'s `OperatorsTest.__init__(self, *args,
                // **kwargs)`, loaded via `unittest`'s `loadTestsFromTestCase`.
                if let Some(idx) = code
                    .varnames
                    .iter()
                    .position(|n| crate::interner::lookup_str(*n) == vararg_name.as_str())
                {
                    if idx < frame.fast_locals.len() {
                        frame.fast_locals[idx] = Some(vararg_val.clone());
                    }
                }
                frame.insert_local(vararg_name.as_str(), vararg_val);
            }
            if npos > named_params.saturating_sub(1) && code.vararg_name.is_none() {
                return Err(PyError::type_error(format!(
                    "{}() takes {} positional argument but {} was given",
                    fname,
                    code.arg_count,
                    npos + 1
                )));
            }
            if npos < named_params.saturating_sub(1) {
                let num_defaults = code.num_defaults;
                for i in npos..named_params.saturating_sub(1) {
                    let idx = i + 1;
                    if idx < code.varnames.len() {
                        let default_idx =
                            num_defaults.saturating_sub(named_params.saturating_sub(1) - i);
                        if default_idx < defaults.len() {
                            let val = defaults[default_idx].clone();
                            frame.fast_locals[idx] = Some(val.clone());
                            frame
                                .insert_local(crate::interner::lookup_str(code.varnames[idx]), val);
                        }
                    }
                }
            }
            // This path never receives keyword arguments at all (its own
            // signature is positional-args-only), so `**kwargs` always ends
            // up empty — but it still needs to be BOUND to an empty dict,
            // not left entirely unset, or a `**kwargs`-taking function body
            // hits the exact same "local variable referenced before
            // assignment" as the vararg case just above the moment it reads
            // its own kwarg parameter (real trigger: the same
            // `OperatorsTest.__init__(self, *args, **kwargs)` scenario,
            // whose body explicitly re-unpacks `**kwargs` into another call).
            if let Some(kwarg_name) = &code.kwarg_name {
                if let Some(idx) = code
                    .varnames
                    .iter()
                    .position(|n| crate::interner::lookup_str(*n) == kwarg_name.as_str())
                {
                    if idx < frame.fast_locals.len() {
                        frame.fast_locals[idx] = Some(py_dict());
                    }
                }
                if !frame.contains_local(kwarg_name) {
                    frame.insert_local(kwarg_name.as_str(), py_dict());
                }
            }
            if std::env::var("RPY_DEBUG_CBM").is_ok() {
                eprintln!(
                    "call_bound_method: fast_locals after setup = {:?}",
                    frame
                        .fast_locals
                        .iter()
                        .map(|v| v.as_ref().map(|x| x.repr()))
                        .collect::<Vec<_>>()
                );
            }
            vm.push_frame(frame);
            vm.execute()
        }
        PyObject::BoundMethod { func, .. } => {
            let mut all_args = vec![self_obj.clone()];
            all_args.extend(args);
            call_bound_method(func.clone(), self_obj, all_args)
        }
        // The single-argument form of the `type` builtin itself (`type(x)`
        // — get x's real type) reaching this path as a plain callable
        // value, e.g. passed as a `key=` function to a native higher-order
        // builtin (`itertools.groupby(data, type)` — real trigger:
        // CPython's own `Lib/statistics.py`). `type` is represented as a
        // real `PyObject::Type{name:"type",..}` here (not a
        // `BuiltinFunction`, unlike most other native callables), so it
        // fell through to the generic "object is not callable" error
        // otherwise. Delegates to the same `builtin_type_of` helper the
        // real CALL-opcode path and `.__class__` both already use — not a
        // new implementation. Other `PyObject::Type` calls (constructing an
        // actual instance, or `type(name, bases, dict)`) still aren't
        // supported through THIS path — only real Python code going
        // through the live VM's own `call_function` gets that; this is
        // scoped to the one case that's actually reachable from a native
        // higher-order builtin's disposable-VM-free call convention.
        PyObject::Type { name, .. } if name == "type" && args.is_empty() => {
            builtin_type_of(&[self_obj])
        }
        _ => Err(PyError::type_error("object is not callable")),
    }
}




pub fn builtin_id(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 1 {
        return Err(PyError::type_error("id() takes exactly one argument"));
    }
    Ok(py_int(args[0].get_id() as i64))
}


pub fn builtin_vars(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 1 {
        return Err(PyError::type_error("vars() takes exactly one argument"));
    }
    let obj = args[0].borrow();
    match &*obj {
        PyObject::Instance { dict, typ } => {
            // NormalDist and other __slots__ classes without __dict__ should
            // raise TypeError for vars() – CPython's `test_statistics`
            // expects `vars(NormalDist(...))` to fail. Our slotted instances
            // still carry a dict for attribute storage, so vars() would
            // incorrectly succeed. Check the type's __slots__.
            if let PyObject::Type { dict: tdict, .. } = &*typ.borrow() {
                if let Some(slots) = tdict.get_str("__slots__") {
                    let is_slots_without_dict = match &*slots.borrow() {
                        PyObject::Tuple(items) => !items.iter().any(|v| v.str() == "__dict__"),
                        PyObject::Str(s) => s != "__dict__",
                        _ => true,
                    };
                    if is_slots_without_dict {
                        return Err(PyError::type_error("vars() argument must have __dict__ attribute"));
                    }
                }
            }
            let mut pd = PyDict::new();
            for (k, v) in dict.iter() {
                pd.set(py_str(k), v.clone())?;
            }
            Ok(PyObjectRef::new(PyObject::Dict(Box::new(pd))))
        }
        PyObject::Module { dict, .. } => {
            let mut pd = PyDict::new();
            for (k, v) in dict.iter() {
                pd.set(py_str(interner::lookup_str(*k)), v.clone())?;
            }
            Ok(PyObjectRef::new(PyObject::Dict(Box::new(pd))))
        }
        _ => Err(PyError::type_error(format!(
            "vars() argument must have __dict__ attribute"
        ))),
    }
}


thread_local! {
    // `isinstance()`/`issubclass()` recurse in PLAIN RUST (calling
    // themselves directly, not through `call_function`) when the
    // `classinfo` argument is a tuple that itself contains tuples —
    // arbitrarily deeply, per real Python semantics (`isinstance(x, (a,
    // (b, (c, ...))))`). `vm.rs`'s `call_function` recursion-depth guard
    // (see its own doc comment) only covers actual Python-level function
    // calls, not this native-Rust recursion, so a deeply/infinitely nested
    // classinfo tuple had no depth limit at all — confirmed general via
    // CPython's own `test_isinstance.py`'s `blowstack()` helper, which
    // builds an ever-growing nested tuple in a `while True:` loop
    // expecting `RecursionError` to interrupt it "eventually": without a
    // guard here, it never does, hanging forever instead of failing fast.
    static ISINSTANCE_RECURSION_DEPTH: std::cell::Cell<usize> = std::cell::Cell::new(0);
}


pub(crate) struct IsinstanceRecursionGuard;


impl IsinstanceRecursionGuard {

    pub(crate) fn enter() -> PyResult<Self> {
        let depth = ISINSTANCE_RECURSION_DEPTH.with(|c| {
            let d = c.get() + 1;
            c.set(d);
            d
        });
        if depth > 1000 {
            ISINSTANCE_RECURSION_DEPTH.with(|c| c.set(c.get() - 1));
            return Err(PyError::recursion_error("maximum recursion depth exceeded"));
        }
        Ok(IsinstanceRecursionGuard)
    }
}


impl Drop for IsinstanceRecursionGuard {

    fn drop(&mut self) {
        ISINSTANCE_RECURSION_DEPTH.with(|c| c.set(c.get().saturating_sub(1)));
    }
}
