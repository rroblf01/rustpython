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
fn own_abc_registry(typ: &PyObjectRef) -> Vec<PyObjectRef> {
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
fn abc_registry_matches_in_subtree(
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


struct IsinstanceRecursionGuard;


impl IsinstanceRecursionGuard {

    fn enter() -> PyResult<Self> {
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


pub fn builtin_isinstance(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let trace = std::env::var("RPY_TRACE_IS").is_ok();
    if trace {
        let o_t = args[0].borrow().type_name();
        let c_t = args[1].borrow().type_name();
        let c_meta = crate::object::metatype_of(&args[1])
            .map(|m| m.borrow().type_name())
            .unwrap_or_else(|| "None".into());
        eprintln!("IS-ENTER obj={} class={} class_meta={}", o_t, c_t, c_meta);
    }
    if args.len() != 2 {
        return Err(PyError::type_error(
            "isinstance() takes exactly 2 arguments",
        ));
    }
    // `isinstance(x, int | str)` — a PEP 604 union used as the second
    // argument checks membership against ANY of its parts, same as the
    // tuple-of-types form just below (real CPython treats `X | Y` and
    // `(X, Y)` identically here). Checked before borrowing `args[1]` as
    // `class` below since `union_args` does its own borrow internally.
    if let Some(members) = crate::modules::union_args(&args[1]) {
        let _guard = IsinstanceRecursionGuard::enter()?;
        for t in &members {
            // A union member can be the literal `None` singleton (`int |
            // None`, matching real CPython's own PEP 604 syntax) rather than
            // `NoneType` itself — `isinstance(x, None)` isn't meaningful
            // (`None` isn't a class), so check against `type(None)` instead.
            let t = if matches!(&*t.borrow(), PyObject::None) {
                builtin_type_of(&[py_none()])?
            } else {
                t.clone()
            };
            let check_args = vec![args[0].clone(), t];
            if builtin_isinstance(&check_args)?.truthy() {
                return Ok(py_bool(true));
            }
        }
        return Ok(py_bool(false));
    }
    let obj = args[0].borrow();
    let class = args[1].borrow();
    // A class with a custom `__instancecheck__` (collections.abc ABCs like
    // Hashable/Iterable/Sized) delegates the check to it.
    if let PyObject::Type { dict, .. } = &*class {
        if let Some(ic) = dict.get_str("__instancecheck__") {
            if !matches!(&*ic.borrow(), PyObject::None) {
                let result = call_bound_method(ic.clone(), args[1].clone(), vec![args[0].clone()])?;
                return Ok(result);
            }
        }
    }
    // Fallback: `__instancecheck__` defined on the class's METACLASS (the
    // standard CPython placement). The dict-level lookup above only fires
    // for the non-standard in-class placement; dynamically built protocol
    // /ABC classes (`_ProtocolMeta('X', (object,), ns)`) carry the hook on
    // their metaclass instead, and — because dynamic class creation with a
    // custom metaclass currently drops the provided namespace into the
    // new Type's dict — the metaclass MRO is the ONLY place the hook is
    // reachable from for them.
    if let Some(meta) = crate::object::metatype_of(&args[1]) {
        if std::env::var("RPY_TRACE_IS").is_ok() {
            eprintln!("IS-TRACE fallback: class={} meta={}",
                      class.type_name(), meta.borrow().type_name());
        }
        if !meta.is(&args[1]) {
            if let Some(f) = crate::object::lookup_dunder_via_mro(&meta, "__instancecheck__") {
                let result =
                    call_bound_method(f, args[1].clone(), vec![args[0].clone()])?;
                return Ok(result);
            }
        }
    }
    // Handle tuple of types: isinstance(x, (type1, type2, ...))
    if let PyObject::Tuple(types) = &*class {
        let _guard = IsinstanceRecursionGuard::enter()?;
        for t in types {
            let check_args = vec![args[0].clone(), t.clone()];
            if builtin_isinstance(&check_args)?.truthy() {
                return Ok(py_bool(true));
            }
        }
        return Ok(py_bool(false));
    }
    match (&*obj, &*class) {
        (
            PyObject::Type { .. },
            PyObject::Type {
                name: class_name, ..
            },
        ) => {
            // isinstance(SomeClass, X): every class is an instance of its
            // metaclass (a custom one if it was built via `metaclass=`,
            // otherwise plain `type` — e.g. `isinstance(Foo, type)` is
            // True for any ordinary class `Foo`). Walk the metaclass's own
            // mro for `X`, exactly like the Instance case above walks the
            // *class's* mro for ordinary instance checks. Deliberately
            // avoids fetching the canonical `type`/`object` singletons via
            // `with_vm_mut` for the no-custom-metaclass fallback below —
            // `isinstance()` runs deep inside live call chains constantly,
            // and grabbing a second aliasing `&mut VirtualMachine` there
            // reliably segfaulted in testing; a plain name comparison
            // (matching how BuiltinFunction-represented native bases are
            // already recognized elsewhere in this function) avoids it.
            if let Some(mt) = metatype_of(&args[0]) {
                if mt.is(&args[1]) {
                    return Ok(py_bool(true));
                }
                if let PyObject::Type { mro, .. } = &*mt.borrow() {
                    for c in mro {
                        if c.is(&args[1]) {
                            return Ok(py_bool(true));
                        }
                    }
                }
                return Ok(py_bool(false));
            }
            Ok(py_bool(class_name == "type" || class_name == "object"))
        }
        (PyObject::Instance { typ, .. }, PyObject::Type { .. }) => {
            // isinstance(obj, Cls): Cls must appear in obj's OWN type's mro
            // — this used to walk `Cls`'s mro checking for `obj`'s exact
            // class name instead (backwards: comparing the wrong object's
            // ancestry, and by name instead of identity), so
            // `isinstance(subclass_instance, ParentClass)` was always False
            // unless the instance's class name happened to literally match
            // one of Cls's own ancestors' names. Confirmed via a minimal,
            // Django-free repro: `isinstance(CharField(), Field)` (a plain
            // one-level subclass check) returning False — this broke every
            // `isinstance(other, Field)`-style guard in real code (Django's
            // `Field.__lt__`/`__eq__`, used by `@total_ordering` and
            // `bisect`-based field ordering during model construction).
            // Direct identity check FIRST, independent of `mro` — a
            // properly-built class (via `default_build_class`) always has
            // itself as `mro[0]`, so this was previously redundant with
            // the mro walk below and easy to miss. But several ad-hoc
            // native `PyObject::Type`s built directly in Rust (this
            // session's own `Fraction`, `namedtuple`-generated classes,
            // `HTTPConnection`, ...) set `mro: vec![]` (empty) since they
            // aren't constructed through the real class-creation
            // machinery — for those, `isinstance(instance, ExactOwnType)`
            // was ALWAYS `False`, even for the most basic possible check,
            // since the type never appeared in its own (empty) mro at
            // all. Confirmed via `isinstance(Fraction(1,2), Fraction)` and
            // `isinstance(a_namedtuple_instance, ThatNamedtupleType)`,
            // both `False` before this fix.
            if typ.is(&args[1]) {
                return Ok(py_bool(true));
            }
            let typ_ref = typ.borrow();
            if let PyObject::Type { mro, .. } = &*typ_ref {
                for c in mro {
                    if c.is(&args[1]) {
                        return Ok(py_bool(true));
                    }
                }
            }
            drop(typ_ref);
            // `ABCMeta.register(subclass)`-style virtual subclass checks
            // (see `.register`'s own doc comment, `get_attribute`'s
            // `PyObject::Type` arm) — `class` (the ABC) records registered
            // classes in its own `_abc_registry` frozenset (read via
            // `own_abc_registry`, NOT `get_attribute` — see its own doc
            // comment for why inherited/MRO-walked registries are wrong
            // here); `obj`'s class (or any of ITS ancestors, for a real
            // subclass of a registered virtual-subclass root) counts too.
            if abc_registry_matches_in_subtree(&args[1], &|registered| {
                typ.is(registered)
                    || matches!(&*typ.borrow(), PyObject::Type { mro, .. } if mro.iter().any(|c| c.is(registered)))
            }) {
                return Ok(py_bool(true));
            }
            Ok(py_bool(false))
        }
        (PyObject::Instance { typ, .. }, _) => {
            let class_name = match &*class {
                PyObject::BuiltinFunction { name, .. } => name.clone(),
                PyObject::Str(s) => s.to_string(),
                _ => class.str(),
            };
            // `class Foo(list): ...` — Foo transparently subclasses a
            // native builtin, so isinstance(foo, list) must also be true.
            if let Some(native_kind) = native_base_of_type(typ) {
                if native_kind == class_name {
                    return Ok(py_bool(true));
                }
            }
            // `class MyError(Exception): ...` — MyError's instances must
            // also be `isinstance(x, Exception)` (and `AttributeError`,
            // `BaseException`, etc, for whatever it really derives from).
            // Builtin exception "classes" are `PyObject::BuiltinFunction`s
            // that never appear in a custom class's own `mro` (only real
            // `PyObject::Type` bases do), so the mro-walk arm just above
            // can't see this relationship at all — without this, NO custom
            // exception subclass was ever recognized as an instance of any
            // of its real (builtin) ancestors, which also broke `except
            // Exception:`/`except SomeBuiltinBase:` for literally any
            // user-defined exception class (CHECK_EXC_MATCH, `vm.rs`, and
            // `builtin_issubclass` below share this exact same gap and fix).
            if let PyObject::BuiltinFunction { .. } = &*class {
                if let Some(base_name) = find_exception_base_name(typ) {
                    if crate::vm::is_exception_subclass(&base_name, &class_name) {
                        return Ok(py_bool(true));
                    }
                }
            }
            Ok(py_bool(
                typ.borrow().type_name() == class_name || class_name == "object",
            ))
        }
        // `isinstance(TypeError, type)` (and similar for any builtin
        // exception "class") — real CPython: every exception class's
        // metaclass is `type`, so this must be True. Since builtin
        // exception classes are represented as plain `PyObject::BuiltinFunction`s
        // here (not `PyObject::Type`s), the generic name-based hierarchy
        // check in the catch-all arm below instead compared this
        // function's own native `type_name()` ("builtin_function_or_method")
        // against "type" in the EXCEPTION hierarchy table (nonsensical —
        // that table is for exception ANCESTRY, not metaclass checks) and
        // always returned False. Real trigger: `unittest`'s own
        // `case.py`'s `_is_subtype`, `isinstance(expected, type) and
        // issubclass(expected, basetype)` — used by `assertRaises(SomeErr)`
        // to validate its argument — always failed, making
        // `self.assertRaises(TypeError)` (or ANY builtin exception class)
        // raise `TypeError: assertRaises() arg 1 must be an exception type
        // or tuple of exception types` instead of actually working.
        (PyObject::BuiltinFunction { name, .. }, PyObject::Type { name: cname, .. })
            if is_builtin_exception_class_name(name) =>
        {
            Ok(py_bool(cname == "type" || cname == "object"))
        }
        _ => {
            let obj_type = args[0].borrow().type_name();
            let class_name = match &*class {
                PyObject::BuiltinFunction { name, .. } => name.clone(),
                PyObject::Str(s) => s.to_string(),
                PyObject::Type { name, .. } => name.clone(),
                _ => class.str(),
            };
            // Direct type name match for built-in types (int, str, list, etc.)
            if obj_type == class_name {
                return Ok(py_bool(true));
            }
            // `bool` is real CPython's one primitive with an actual
            // inheritance relationship to another primitive
            // (`bool.__bases__ == (int,)`) — `isinstance(True, int)` and
            // `isinstance(True, object)` must both be `True`. A `bool`
            // value here is always the PRIMITIVE `PyObjectRef::SmallBool`/
            // `PyObject::Bool` shape (never a `PyObject::Instance`, since
            // `bool` can't be subclassed — see `default_build_class`'s
            // explicit block for that), so this can't be expressed via the
            // generic mro-walk arms above (which all key off an `Instance`'s
            // `typ`) — a narrow, direct name check is the simplest correct
            // fix, mirroring this function's existing style for other
            // primitive/name-based special cases.
            if obj_type == "bool" && (class_name == "int" || class_name == "object") {
                return Ok(py_bool(true));
            }
            // See `builtin_isinstance`'s other `_abc_registry` fallback —
            // this one covers a PRIMITIVE (inline `SmallInt`/`SmallStr`/...
            // or boxed `Int`/`Str`/...) checked against an ABC that
            // registered the matching builtin type name (real trigger:
            // `numbers.py`'s own `Integral.register(int)`/`Real.register
            // (float)`/`Complex.register(complex)` — needed for
            // `isinstance(5, numbers.Integral)` to work at all).
            if let PyObject::Type { .. } = &*class {
                if abc_registry_matches_in_subtree(&args[1], &|registered| {
                    let registered_name = match &*registered.borrow() {
                        PyObject::BuiltinFunction { name, .. } => name.clone(),
                        PyObject::Type { name, .. } => name.clone(),
                        _ => return false,
                    };
                    registered_name == obj_type
                }) {
                    return Ok(py_bool(true));
                }
            }
            // Exception hierarchy — but only for objects that can actually
            // BE exceptions: real `PyObject::Exception` instances (builtin
            // or user-defined subclass), or builtin exception CLASS names.
            // `is_exception_subclass` maps unknown type names to `Exception`
            // by default (so user-defined `class MyError(Exception)` bodies
            // resolve correctly), which wrongly made ANY primitive value —
            // `isinstance('x', BaseException)`, `isinstance([], ValueError)`
            // — resolve through the exception table and return True. Real
            // trigger: `Lib/test/support/os_helper.py`'s `FakePath.__fspath__`
            // guards on `isinstance(self.path, BaseException)`, and a str
            // path incorrectly took the exception-raising branch, so every
            // `os.path.*`/`open()` call handed a `FakePath` failed.
            if matches!(&*obj, PyObject::Exception { .. })
                || is_builtin_exception_class_name(&obj_type)
            {
                return Ok(py_bool(crate::vm::is_exception_subclass(
                    &obj_type,
                    &class_name,
                )));
            }
            Ok(py_bool(false))
        }
    }
}


/// Real `open()`/`os.*` path arguments accept `str`, `bytes`, or any
/// `os.PathLike` — but `PyObjectRef::str()` on a `PyObject::Bytes` value
/// deliberately returns Python's own `repr()`-style `"b'...'"` form (matching
/// real CPython's `str(b"x")` semantics — NOT a bug on its own), which is
/// exactly the WRONG thing to feed to a filesystem call. Real trigger:
/// CPython's own `dbm/dumb.py` (via `os.fsencode`), which builds its
/// `.dat`/`.dir`/`.bak` filenames as `bytes` and passes them straight to
/// `io.open()` — `open(b"/tmp/x.dat", "w")` tried to open a file literally
/// named `b'/tmp/x.dat'` (quotes, `b` prefix and all), which obviously
/// doesn't exist, raising a confusing `OSError` instead of writing to the
/// intended path.
pub(crate) fn path_arg_to_string(obj: &PyObjectRef) -> String {
    // PEP 519 `os.PathLike` protocol: any object with a `__fspath__()`
    // method (real code: `pathlib.Path`, and plenty of test-only wrappers
    // like `Lib/test/support/os_helper.py`'s `FakePath`) must have THAT
    // called to get the real path, instead of falling through to
    // `.str()` — which, for a plain custom class instance with no
    // `__str__` override, produces its REPR (`<FakePath '/tmp/xxx'>`,
    // literally including the wrapper's own class name and quoting) rather
    // than the real path string, so every `open()`/`os.*` call given such
    // a wrapped path silently tried to open a nonexistent file named after
    // its own repr instead. Confirmed via `test_dbm.py::test_whichdb`,
    // which explicitly exercises `FakePath`-wrapped paths.
    let f = {
        let o = obj.borrow();
        if let PyObject::Instance { typ, .. } = &*o {
            lookup_dunder_via_mro(typ, "__fspath__")
        } else {
            None
        }
    };
    if let Some(f) = f {
        if let Ok(result) = call_bound_method(f, obj.clone(), vec![]) {
            return path_arg_to_string(&result);
        }
    }
    if let PyObject::Bytes(b) = &*obj.borrow() {
        String::from_utf8_lossy(b).into_owned()
    } else {
        obj.str()
    }
}


pub fn builtin_open(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error(
            "open() missing required argument 'file'",
        ));
    }
    // A keyword call (e.g. the extremely common `open(path, encoding="utf-8")`,
    // with NO explicit `mode`) reaches every plain `BuiltinFunction` with its
    // keywords packed into a dict APPENDED as the last positional arg (see
    // `vm.rs`'s `call_function`) — this was read directly as `args[1]` (the
    // "mode" position) whenever ANY keyword was passed, regardless of
    // whether `mode` itself was one of them. `dict.str()` on that packed
    // kwargs dict produced something like `"{'encoding': 'utf-8'}"`, which
    // contains none of 'r'/'w'/'a'/'+' — so NO read/write/append flag ever
    // got set, and the file open failed with a raw, confusing `OSError:
    // must specify at least one of read, write, or append access` instead
    // of just opening for reading (the real, correct default). Confirmed
    // via `test_baseexception.py::test_inheritance`'s own `open(path,
    // encoding="utf-8")` call. Now separates a trailing kwargs dict (if
    // present — `open()`'s real parameters are never legitimately a dict
    // themselves, so this is unambiguous) and reads `mode` from either the
    // second positional arg or the `mode` keyword, defaulting to `"r"`.
    let (pos_args, kwargs) = match args.last() {
        Some(last) if matches!(&*last.borrow(), PyObject::Dict(_)) => {
            (&args[..args.len() - 1], Some(last))
        }
        _ => (args, None),
    };
    let filename = path_arg_to_string(&pos_args[0]);
    let mode = if pos_args.len() > 1 {
        pos_args[1].str()
    } else if let Some(kw) = kwargs {
        if let PyObject::Dict(d) = &*kw.borrow() {
            d.get(&py_str("mode"))
                .ok()
                .flatten()
                .map(|v| v.str())
                .unwrap_or_else(|| "r".to_string())
        } else {
            "r".to_string()
        }
    } else {
        "r".to_string()
    };
    // A trailing `+` ("r+"/"w+"/"a+", real CPython's "and updating" suffix)
    // means the file is opened for BOTH reading and writing — was
    // completely ignored here, so "rb+" (read-write, don't truncate, don't
    // create — the exact mode `dbm/dumb.py` uses to append new values to
    // its own data file) only ever opened for reading, and a subsequent
    // `f.write(...)` failed with a raw OS-level "Bad file descriptor"
    // instead of writing.
    let has_plus = mode.contains('+');
    // Mode `'x'` = exclusive create (real CPython's third create flag,
    // alongside `'w'` create+truncate and `'a'` create+append): creates the
    // file but FAILS with `FileExistsError` if it already exists. Was
    // completely unrecognized — `open(path, 'xb')` (the very common "write
    // this file only if I'm not clobbering something" idiom, used all over
    // CPython's own test suite) set NO read/write flag at all, failing with
    // a raw `OSError: must specify at least one of read, write, or append
    // access`. It implies write, exactly like `'w'`.
    let has_x = mode.contains('x');
    let mut opts = std::fs::File::options();
    opts.read(mode.contains('r') || has_plus)
        .write(mode.contains('w') || mode.contains('a') || has_plus || has_x)
        .append(mode.contains('a'))
        .create(mode.contains('w') || mode.contains('a') || has_x)
        .truncate(mode.contains('w'));
    if has_x {
        opts.create_new(true);
    }
    let file = opts
        .open(&filename)
        .map_err(|e| PyError::os_error_from_io(&e))?;
    let binary = mode.contains('b');
    Ok(PyObjectRef::new(PyObject::File {
        file: std::rc::Rc::new(std::cell::RefCell::new(file)),
        name: filename,
        binary,
        pending: std::rc::Rc::new(std::cell::RefCell::new(Vec::new())),
        closed: false,
    }))
}


// Python-semantics modulo for `BigInt` (result takes the SIGN OF THE
// DIVISOR, unlike Rust's `%`, which takes the sign of the dividend) — needed
// by `builtin_pow`'s 3-arg form below, whose test coverage explicitly checks
// negative moduli (`test_pow.py::test_negative_exponent` sweeps `m` from
// -50 to 49).


pub fn builtin_issubclass(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 2 {
        return Err(PyError::type_error(
            "issubclass() takes exactly 2 arguments",
        ));
    }
    // `issubclass(cls, int | str)` — same PEP 604 union-membership check as
    // `builtin_isinstance`'s matching case just above.
    if let Some(members) = crate::modules::union_args(&args[1]) {
        let _guard = IsinstanceRecursionGuard::enter()?;
        for t in &members {
            let t = if matches!(&*t.borrow(), PyObject::None) {
                builtin_type_of(&[py_none()])?
            } else {
                t.clone()
            };
            let check_args = vec![args[0].clone(), t];
            if builtin_issubclass(&check_args)?.truthy() {
                return Ok(py_bool(true));
            }
        }
        return Ok(py_bool(false));
    }
    // Custom __subclasscheck__ (e.g. os.PathLike virtual subclass)
    {
        let maybe_sc = {
            let base_b = args[1].borrow();
            if let PyObject::Type { dict, .. } = &*base_b {
                dict.get_str("__subclasscheck__").cloned()
            } else {
                None
            }
        };
        if let Some(sc) = maybe_sc {
            if !matches!(&*sc.borrow(), PyObject::None) {
                let result = call_bound_method(sc, args[1].clone(), vec![args[0].clone()])?;
                return Ok(result);
            }
        }
        if let Some(meta) = crate::object::metatype_of(&args[1]) {
            if !meta.is(&args[1]) {
                if let Some(f) = crate::object::lookup_dunder_via_mro(&meta, "__subclasscheck__") {
                    let result = call_bound_method(f, args[1].clone(), vec![args[0].clone()])?;
                    return Ok(result);
                }
            }
        }
    }
    // Handle tuple of types: issubclass(cls, (type1, type2, ...))
    let base = args[1].borrow();
    if let PyObject::Tuple(types) = &*base {
        let _guard = IsinstanceRecursionGuard::enter()?;
        for t in types {
            let check_args = vec![args[0].clone(), t.clone()];
            if builtin_issubclass(&check_args)?.truthy() {
                return Ok(py_bool(true));
            }
        }
        return Ok(py_bool(false));
    }
    let cls = args[0].borrow();
    drop(base);
    let base = args[1].borrow();
    match (&*cls, &*base) {
        (PyObject::Type { mro: cls_mro, .. }, PyObject::Type { .. }) => {
            // Direct identity check first — same fix, same reason, as
            // `builtin_isinstance`'s matching arm just above: several
            // ad-hoc native `PyObject::Type`s (`Fraction`, `namedtuple`-
            // generated classes, ...) have an empty `mro`, so
            // `issubclass(X, X)` was `False` even for the most basic
            // possible check.
            if args[0].is(&args[1]) {
                return Ok(py_bool(true));
            }
            let base_tn = base.type_name();
            for c in cls_mro {
                if c.borrow().type_name() == base_tn {
                    return Ok(py_bool(true));
                }
            }
            // See `builtin_isinstance`'s matching fallback for
            // `ABCMeta.register`-style virtual subclass checks.
            if abc_registry_matches_in_subtree(&args[1], &|registered| {
                cls_mro.iter().any(|c| c.is(registered))
            }) {
                return Ok(py_bool(true));
            }
            Ok(py_bool(false))
        }
        (PyObject::Type { mro: cls_mro, .. }, _) => {
            // Non-Type second argument: compare by name. `.str()` on a
            // BuiltinFunction (how builtin exception "classes" like
            // `Exception` are represented) returns its full repr
            // (`<built-in function Exception>`), not the bare name — must
            // extract that explicitly or every name comparison below
            // (including the exception-ancestry fix just past the mro
            // walk) silently never matches.
            let base_name = match &*base {
                PyObject::BuiltinFunction { name, .. } => name.clone(),
                _ => base.str(),
            };
            if base_name == "object" {
                return Ok(py_bool(true));
            }
            for c in cls_mro {
                if c.borrow().type_name() == base_name {
                    return Ok(py_bool(true));
                }
            }
            // `issubclass(MyError, Exception)` where MyError is a real
            // user-defined `class MyError(Exception): ...` — Exception (a
            // `BuiltinFunction`, not a `Type`) never appears in MyError's own
            // `mro`, so the walk above can't see this. Same gap/fix as
            // isinstance()'s Instance/BuiltinFunction arm just above.
            if matches!(&*base, PyObject::BuiltinFunction { .. }) {
                if let Some(base_exc_name) = find_exception_base_name(&args[0]) {
                    if crate::vm::is_exception_subclass(&base_exc_name, &base_name) {
                        return Ok(py_bool(true));
                    }
                }
            }
            Ok(py_bool(false))
        }
        // Built-in exception "classes" (e.g. KeyError, ValueError) are
        // represented as BuiltinFunction constructors rather than Type
        // objects — resolve ancestry by name via the same table `except`
        // and isinstance() use, instead of only accepting real Type values.
        // A BuiltinFunction that is NOT a recognized exception class is not
        // a class at all (real CPython: issubclass(abs, X) raises TypeError)
        // — without this gate, `is_exception_subclass`'s catch-all mapped
        // ANY BuiltinFunction name (abs, print, ...) to Exception, so
        // `issubclass(abs, BaseException)` returned True.
        (PyObject::BuiltinFunction { name: cls_name, .. }, _) => {
            if !is_builtin_exception_class_name(cls_name) {
                return Err(PyError::type_error("issubclass() arg 1 must be a class"));
            }
            let base_name = match &*base {
                PyObject::BuiltinFunction { name, .. } => name.clone(),
                PyObject::Str(s) => s.to_string(),
                PyObject::Type { name, .. } => name.clone(),
                _ => base.str(),
            };
            // Everything (including every exception "class") is a subclass of
            // `object` — `issubclass(Exception, object)` must be True
            // (test_baseexception::test_builtins_new_style).
            if base_name == "object" {
                return Ok(py_bool(true));
            }
            if crate::vm::is_exception_subclass(cls_name, &base_name) {
                return Ok(py_bool(true));
            }
            // See `builtin_isinstance`'s `_abc_registry` fallback — this
            // covers `issubclass(int, numbers.Integral)`-style checks
            // where the SUBCLASS side (`int`/`float`/`complex`/...) is
            // itself a `BuiltinFunction`, not a `Type`. Real trigger:
            // `numbers.py`'s own `Integral.register(int)`/`Real.register
            // (float)`/`Complex.register(complex)`.
            if let PyObject::Type { .. } = &*base {
                if abc_registry_matches_in_subtree(&args[1], &|registered| {
                    let registered_name = match &*registered.borrow() {
                        PyObject::BuiltinFunction { name, .. } => name.clone(),
                        PyObject::Type { name, .. } => name.clone(),
                        _ => return false,
                    };
                    &registered_name == cls_name
                }) {
                    return Ok(py_bool(true));
                }
            }
            Ok(py_bool(false))
        }
        // `Opcode::WITH_EXIT` (`vm.rs`) resolves a raised `PyObject::Exception`'s
        // `exc_type` argument to `__exit__` by looking its `typ` name up in the
        // CURRENT frame's builtins — which only ever holds the ~70 core
        // exception names (`add_exc_type!` in `create_builtins`), never a
        // module-scoped custom exception like `struct.error`/`pickle.PickleError`
        // (those live only in their own module's dict, not global builtins).
        // When that lookup misses, it falls back to a bare `PyObject::Str`
        // holding just the name, as a last-resort placeholder — so
        // `issubclass(exc_type, struct.error)` inside `unittest`'s own
        // `_AssertRaisesBaseContext.__exit__` reached here with a plain string
        // for `cls`. This codebase already treats built-in/module exception
        // "classes" as interchangeable by name everywhere else (the
        // `BuiltinFunction`/`Type`/`Str` arms just above), so extending that
        // same name-based comparison to a bare string `cls` here is consistent
        // for that INTERNAL fallback specifically — but a real user calling
        // `issubclass("hello", BaseException)` must still get real Python's
        // `TypeError: issubclass() arg 1 must be a class` (previously this
        // arm accepted ANY string unconditionally, so a plain string that
        // merely happened to arrive here — confirmed via CPython's own
        // `test_baseexception.py`, whose `test_inheritance`/`test_catch_string`
        // pass arbitrary strings including plain object values from
        // `builtins.__dict__` — silently returned `False`/`True` by name
        // comparison instead of raising). Gated on `cls_name` actually being
        // one of the recognized builtin/module exception names — every
        // legitimate internal fallback string is drawn from exactly that set
        // (`add_exc_type!`'s own names, or a module exception registered via
        // `is_builtin_exception_class_name`), so an unrecognized string can
        // only be genuine user input, not this internal fallback.
        (PyObject::Str(cls_name), _) if is_builtin_exception_class_name(cls_name) => {
            let base_name = match &*base {
                PyObject::BuiltinFunction { name, .. } => name.clone(),
                PyObject::Str(s) => s.to_string(),
                PyObject::Type { name, .. } => name.clone(),
                _ => base.str(),
            };
            Ok(py_bool(crate::vm::is_exception_subclass(
                cls_name, &base_name,
            )))
        }
        _ => {
            if std::env::var("RPY_DEBUG_ISSUBCLASS").is_ok() {
                eprintln!(
                    "issubclass() FAIL: arg0={:?}/{} arg1={:?}/{}",
                    cls.type_name(),
                    cls.repr(),
                    base.type_name(),
                    base.repr()
                );
            }
            Err(PyError::type_error("issubclass() arg 1 must be a class"))
        }
    }
}


pub fn builtin_help(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        println!("Welcome to RustPython 0.1.0!");
        println!();
        println!("Available built-in functions:");
        println!("  abs()  all()  any()  ascii()  bin()  bool()  breakpoint()");
        println!("  bytearray()  bytes()  callable()  chr()  compile()  delattr()");
        println!("  dict()  dir()  divmod()  enumerate()  eval()  exec()  exit()");
        println!("  filter()  float()  format()  frozenset()  getattr()  globals()");
        println!("  hasattr()  hash()  help()  hex()  id()  input()  int()");
        println!("  isinstance()  issubclass()  iter()  len()  list()  locals()");
        println!("  map()  max()  memoryview()  min()  next()  object()  oct()");
        println!("  open()  ord()  pow()  print()  property()  range()  repr()");
        println!("  reversed()  round()  set()  setattr()  slice()  sorted()");
        println!("  staticmethod()  str()  sum()  super()  tuple()  type()  vars()");
        println!("  zip()");
        println!();
        println!("Available error types:");
        println!("  BaseException  Exception  TypeError  ValueError  ZeroDivisionError");
        println!("  NameError  AttributeError  IndexError  KeyError  RuntimeError");
        println!("  StopIteration  AssertionError  OSError  ImportError  LookupError");
        println!("  ArithmeticError  OverflowError  NotImplementedError  RecursionError");
        println!("  KeyboardInterrupt  SystemExit  ModuleNotFoundError  FileNotFoundError");
        println!("  PermissionError  UnicodeDecodeError  UnicodeEncodeError");
        println!();
        println!("Type help(object) for information about a specific object.");
    } else {
        let obj = args[0].borrow();
        match &*obj {
            PyObject::Type { name, dict, .. } => {
                println!("Help on class {}:", name);
                if let Some(doc) = dict.get_str("__doc__") {
                    println!("  {}", doc.str());
                }
                println!();
                println!("Methods:");
                for (key, val) in dict.iter() {
                    if matches!(
                        &*val.borrow(),
                        PyObject::Function(_) | PyObject::BuiltinFunction { .. }
                    ) {
                        println!("  {}()", interner::lookup_str(*key));
                    }
                }
            }
            PyObject::Function(ref f) => {
                let name = &f.code.name;
                let dict = &f.dict;
                println!("Help on function {}:", name);
                if let Some(doc) = dict.get("__doc__") {
                    println!("  {}", doc.str());
                }
            }
            PyObject::BuiltinFunction { name, .. } => {
                println!("Help on built-in function {}:", name);
            }
            _ => {
                println!("Help on {}:", obj.type_name());
                println!("  Type: {}", obj.type_name());
            }
        }
    }
    Ok(py_none())
}
