use crate::interner::{self, StrId};
use crate::object::*;
use crate::vm::helpers::formal_param_index;
use crate::vm::VirtualMachine;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

impl VirtualMachine {
    pub(crate) fn handle_py_function_call(
        &mut self,
        callable: &PyObjectRef,
        args: Vec<PyObjectRef>,
        keywords: Vec<(String, PyObjectRef)>,
    ) -> Option<PyResult<PyObjectRef>> {
        let inner_f = match &*callable.borrow() {
            PyObject::Function(ref inner_f) => inner_f.clone(),
            _ => return None,
        };
        let result: PyResult<PyObjectRef> = (|| {
            let code = &inner_f.code;
            let func_globals = &inner_f.globals;
            let defaults = &inner_f.defaults;
            let closure = &inner_f.closure;
            let jit_ptr = &inner_f.jit_ptr;
            let jit_consts = &inner_f.jit_consts;
            // Try JIT compiled execution (fast path for hot functions)
            #[cfg(feature = "jit")]
            if defaults.is_empty()
                && keywords.is_empty()
                && !crate::cycle_gc::IN_FINALIZER.with(std::cell::Cell::get)
            {
                const SENTINEL_FAILED: usize = 1;
                let jp = jit_ptr.get();
                if jp == SENTINEL_FAILED {
                    // A previous compile attempt failed — stick with the
                    // interpreter (don't retry on every call).
                } else {
                    if jp == 0 {
                        // First call: compile now and run the result
                        // immediately (this VM's tests call most functions
                        // once, so deferring to the second call would leave
                        // them interpreted forever).
                        let compiled_fn = self.jit.borrow_mut().compile(code);
                        match compiled_fn {
                            Some(compiled_fn) => {
                                let precomputed = crate::jit::JitCompiler::precompute_for_jit(
                                    code,
                                    func_globals,
                                    &self.builtins,
                                );
                                jit_ptr.set(compiled_fn as usize);
                                *jit_consts.borrow_mut() = precomputed;
                            }
                            None => {
                                jit_ptr.set(SENTINEL_FAILED);
                            }
                        }
                    }
                    let jp = jit_ptr.get();
                    if jp != 0 && jp != SENTINEL_FAILED {
                        // SAFETY: `jp` was just produced by
                        // `self.jit.borrow_mut().compile(code)` above (or on
                        // a prior call for the same `code`), which only ever
                        // emits machine code matching this exact
                        // `extern "C"` signature — the JIT codegen in
                        // jit.rs is the sole producer of values stored in
                        // `jit_ptr`.
                        let func_ptr: extern "C" fn(
                            *const PyObjectRef,
                            usize,
                            *const PyObjectRef,
                            *mut PyObjectRef,
                        ) = unsafe { std::mem::transmute(jp) };
                        let n = args.len().min(code.arg_count as usize);
                        let mut fast_locals: Vec<PyObjectRef> = Vec::with_capacity(n);
                        for i in 0..n {
                            fast_locals.push(args[i].clone());
                        }
                        let consts = jit_consts.borrow();
                        let mut result = PyObjectRef::None;
                        let _guard = crate::jit::set_jit_globals(func_globals.clone());
                        func_ptr(
                            fast_locals.as_ptr(),
                            fast_locals.len(),
                            consts.as_ptr(),
                            &mut result,
                        );
                        return Ok(result);
                    }
                }
            }

            // Try simple execution without Frame creation
            if defaults.is_empty() && keywords.is_empty() {
                if let Some(result) = Self::try_exec_simple(code, &args) {
                    return result;
                }
            }
            // A Python-level function call here recurses through actual
            // Rust call frames (`call_function` -> `execute()` ->
            // `execute_inner` -> `execute_instruction`'s `CALL` handling ->
            // `call_function` -> ...), with no equivalent of CPython's own
            // `sys.getrecursionlimit()` check anywhere — so unbounded
            // Python recursion (a plain accidental bug in user code, not
            // some contrived edge case) previously overflowed the REAL
            // native thread stack and hard-aborted the whole process
            // (`fatal runtime error: stack overflow`) instead of raising a
            // catchable `RecursionError`, exactly like real CPython does.
            // Confirmed general via the simplest possible repro (`def
            // f(n): return f(n+1)` called once) and via CPython's own
            // `test_isinstance.py`'s deliberate recursion-limit tests.
            // Reads `self.recursion_limit` (default matches real CPython's
            // `sys.getrecursionlimit()`, 1000 — see its own doc comment).
            // Made safe by `main.rs` running everything on a dedicated,
            // much larger-than-default stack sized with headroom to spare
            // even at the default limit.
            if self.frames.len() >= self.recursion_limit {
                return Err(PyError::recursion_error("maximum recursion depth exceeded"));
            }
            let func_globals = func_globals.clone();
            let defaults = defaults.clone();
            let code_rc = Rc::new(code.clone());
            let mut new_frame = self.acquire_frame(
                Rc::clone(&code_rc),
                func_globals,
                Rc::clone(&self.builtins),
                None,
            );
            new_frame.closure = Box::new(closure.clone());
            let code = code;

            let npos = args.len();
            let named_params = code.arg_count;
            let fname = interner::lookup_str(code.name).to_string();

            fn format_missing_names(names: &[String]) -> String {
                match names.len() {
                    0 => String::new(),
                    1 => format!("'{}'", names[0]),
                    2 => format!("'{}' and '{}'", names[0], names[1]),
                    _ => {
                        let (last, rest) = names.split_last().unwrap();
                        let joined = rest
                            .iter()
                            .map(|n| format!("'{}'", n))
                            .collect::<Vec<_>>()
                            .join(", ");
                        format!("{}, and '{}'", joined, last)
                    }
                }
            }

            // Real Python raises `TypeError` immediately when more positional
            // arguments are given than the function accepts (and it has no
            // `*args` to absorb the excess) — this whole argument-binding
            // block had NO validation of any kind before this fix: too many
            // positional args were silently dropped, missing required args
            // were never detected (the function body would just hit
            // `LOAD_FAST unbound` chaos or read `None`), unexpected keyword
            // arguments were silently inserted as a throwaway local name,
            // and a keyword colliding with an already-positionally-filled
            // parameter silently overwrote it instead of raising. Found via
            // CPython's own `test_call.py`
            // (`TestErrorMessagesUseQualifiedName`/`CFunctionCallsErrorMessages`),
            // whose whole point is exercising exactly these error paths —
            // every single one of them was a real, silent correctness bug
            // affecting EVERY user-defined function call in the interpreter.
            if npos > named_params && code.vararg_name.is_none() {
                self.release_frame(new_frame);
                let num_defaults = code.num_defaults;
                let min_required = named_params.saturating_sub(num_defaults);
                // CPython's arg-count TypeError grammar: the noun agrees
                // with the count, the verb agrees with the TOTAL (npos +
                // keyword-only) — e.g. "but 1 positional argument (and 1
                // keyword-only argument) were given". Matches the doctest in
                // test_extcall.py exactly.
                let noun = |n: usize| if n == 1 { "argument" } else { "arguments" };
                // count how many passed keywords target kwonly params, for the
                // extended error message "and N keyword-only arguments"
                let kwonly_given = if code.kwonlyarg_count > 0 && !keywords.is_empty() {
                    let kwonly_start_tmp =
                        code.arg_count + if code.vararg_name.is_some() { 1 } else { 0 };
                    let kwonly_names = &code.varnames
                        [kwonly_start_tmp..kwonly_start_tmp + code.kwonlyarg_count];
                    keywords
                        .iter()
                        .filter(|(k, _)| {
                            kwonly_names.iter().any(|&n| crate::interner::intern_eq(n, k))
                        })
                        .count()
                } else {
                    0
                };
                let msg = if kwonly_given > 0 {
                    // CPython 3.14: "takes X positional arguments but Y positional arguments (and Z keyword-only arguments) were given"
                    format!(
                        "{}() takes {} positional {} but {} positional {} (and {} keyword-only {}) were given",
                        fname,
                        named_params,
                        noun(named_params),
                        npos,
                        noun(npos),
                        kwonly_given,
                        noun(kwonly_given),
                    )
                } else if num_defaults == 0 {
                    format!(
                        "{}() takes {} positional {} but {} {} given",
                        fname,
                        named_params,
                        noun(named_params),
                        npos,
                        if npos == 1 { "was" } else { "were" }
                    )
                } else {
                    let verb = if npos > 1 { "were" } else { "was" };
                    format!(
                        "{}() takes from {} to {} positional arguments but {} {} given",
                        fname,
                        min_required,
                        named_params,
                        npos,
                        verb
                    )
                };
                return Err(PyError::type_error(msg));
            }

            // Assign positional args to named parameters
            for i in 0..npos.min(named_params) {
                let name_clone = new_frame.code.varnames[i].to_string();
                new_frame.insert_local(&name_clone, args[i].clone());
                if i < new_frame.fast_locals.len() {
                    new_frame.fast_locals[i] = Some(args[i].clone());
                }
            }

            // Pack excess positional args into *args
            if let Some(vararg_name) = &code.vararg_name {
                let mut extra = Vec::new();
                for i in named_params..npos {
                    extra.push(args[i].clone());
                }
                let vararg_val = py_tuple(extra);
                if let Some(idx) = new_frame
                    .code
                    .varnames
                    .iter()
                    .position(|&n| crate::interner::intern_eq(n, vararg_name))
                {
                    if idx < new_frame.fast_locals.len() {
                        new_frame.fast_locals[idx] = Some(vararg_val.clone());
                    }
                }
                new_frame.insert_local(&vararg_name, vararg_val);
            }

            // Apply defaults for missing positional params
            if npos < named_params {
                let num_defaults = code.num_defaults;
                // Parameters are split into two groups: those WITHOUT defaults (non-defaulted),
                // and those WITH defaults (defaulted). self (index 0) is never defaulted.
                // defaulted params start at index (named_params - num_defaults)
                let first_default = named_params - num_defaults;
                for i in npos..named_params {
                    if i >= first_default {
                        let default_idx = i - first_default;
                        let name_clone = new_frame.code.varnames[i].to_string();
                        let val = if default_idx < defaults.len() {
                            defaults[default_idx].clone()
                        } else {
                            py_none()
                        };
                        new_frame.insert_local(&name_clone, val.clone());
                        if i < new_frame.fast_locals.len() {
                            new_frame.fast_locals[i] = Some(val);
                        }
                    }
                }
            }

            // Handle **kwargs
            let kwonly_start = code.arg_count + if code.vararg_name.is_some() { 1 } else { 0 };
            let positional_filled = npos.min(named_params);
            if let Some(kwarg_name) = &code.kwarg_name {
                let kw_dict = py_dict();
                for (key, value) in &keywords {
                    if let Some(idx) = formal_param_index(
                        &new_frame.code.varnames,
                        code.arg_count,
                        code.posonlyarg_count,
                        code.kwonlyarg_count,
                        kwonly_start,
                        key,
                    ) {
                        // A keyword targeting a positional-only param goes
                        // into **kwargs (real Python: `f(42, a=1)` with `a`
                        // posonly lands in kwargs, never on the param).
                        if idx < code.posonlyarg_count {
                            if let PyObject::Dict(ref mut dict) = &mut *kw_dict.borrow_mut() {
                                dict.set(py_str(key), value.clone())?;
                            }
                            continue;
                        }
                        // A keyword targeting a formal parameter that ALREADY
                        // received a positional value — real Python's
                        // `TypeError: ...() got multiple values for argument
                        // '...'`, previously silently overwritten.
                        if idx < positional_filled {
                            self.release_frame(new_frame);
                            return Err(PyError::type_error(format!(
                                "{}() got multiple values for argument '{}'",
                                fname, key
                            )));
                        }
                        new_frame.insert_local(&key, value.clone());
                        if idx < new_frame.fast_locals.len() {
                            new_frame.fast_locals[idx] = Some(value.clone());
                        }
                    } else {
                        if let PyObject::Dict(ref mut dict) = &mut *kw_dict.borrow_mut() {
                            // A key supplied more than once — via `**{k: v}`
                            // expansion AND an explicit keyword (or twice via
                            // **) — is `TypeError: ...() got multiple values
                            // for keyword argument 'k'` (test_extcall's
                            // doctest: `f(1, 2, **{'a': -1}, a=4, c=6)`).
                            if dict.get(&py_str(key)).ok().flatten().is_some() {
                                self.release_frame(new_frame);
                                return Err(PyError::type_error(format!(
                                    "{}() got multiple values for keyword argument '{}'",
                                    fname, key
                                )));
                            }
                            dict.set(py_str(key), value.clone())?;
                        }
                    }
                }
                if let Some(idx) = new_frame
                    .code
                    .varnames
                    .iter()
                    .position(|n| crate::interner::lookup_str(*n) == kwarg_name.as_str())
                {
                    if idx < new_frame.fast_locals.len() {
                        new_frame.fast_locals[idx] = Some(kw_dict.clone());
                    }
                }
                new_frame.insert_local(kwarg_name.as_str(), kw_dict);
            } else {
                // No **kwargs: keyword args must still bind to the matching
                // named parameter's FAST local slot (LOAD_FAST reads
                // fast_locals, not the insert_local name dict — missing this
                // meant `f(1, somekw=True)` left `somekw` as None in
                // fast_locals, raising "referenced before assignment" the
                // moment the function body read it), matching the
                // **kwargs branch above. A keyword matching no formal
                // parameter, or one that already got a positional value,
                // must raise `TypeError` — previously silently accepted as
                // either a no-op or a throwaway local-name insertion the
                // function body never referenced.
                // With no **kwargs, ALL keywords targeting positional-only
                // params are reported together (real Python's
                // "got some positional-only arguments passed as keyword
                // arguments: 'a, b'").
                let posonly_keywords: Vec<&String> = keywords
                    .iter()
                    .filter_map(|(k, _)| {
                        formal_param_index(
                            &new_frame.code.varnames,
                            code.arg_count,
                            code.posonlyarg_count,
                            code.kwonlyarg_count,
                            kwonly_start,
                            k,
                        )
                        .filter(|idx| *idx < code.posonlyarg_count)
                        .map(|_| k)
                    })
                    .collect();
                if !posonly_keywords.is_empty() {
                    self.release_frame(new_frame);
                    let names = posonly_keywords
                        .iter()
                        .map(|k| k.as_str())
                        .collect::<Vec<_>>()
                        .join(", ");
                    return Err(PyError::type_error(format!(
                        "{}() got some positional-only arguments passed as keyword arguments: '{}'",
                        fname, names
                    )));
                }
                for (key, value) in &keywords {
                    match formal_param_index(
                        &new_frame.code.varnames,
                        code.arg_count,
                        code.posonlyarg_count,
                        code.kwonlyarg_count,
                        kwonly_start,
                        key,
                    ) {
                        Some(idx) if idx < code.posonlyarg_count => {
                            // Unreachable (pre-scanned above) — keep for safety.
                            self.release_frame(new_frame);
                            return Err(PyError::type_error(format!("{}() got some positional-only arguments passed as keyword arguments: '{}'", fname, key)));
                        }
                        Some(idx) if idx < positional_filled => {
                            self.release_frame(new_frame);
                            return Err(PyError::type_error(format!(
                                "{}() got multiple values for argument '{}'",
                                fname, key
                            )));
                        }
                        Some(idx) => {
                            if idx < new_frame.fast_locals.len() {
                                new_frame.fast_locals[idx] = Some(value.clone());
                            }
                            new_frame.insert_local(&key, value.clone());
                        }
                        None => {
                            self.release_frame(new_frame);
                            return Err(PyError::type_error(format!(
                                "{}() got an unexpected keyword argument '{}'",
                                fname, key
                            )));
                        }
                    }
                }
            }

            // Apply defaults for still-unbound keyword-only params (CPython's
            // __kwdefaults__ equivalent) — must run after explicit keyword
            // binding above, since only truly-unbound kwonly slots should
            // get their default. Defaults for kwonly params live in
            // `defaults` right after the positional ones (see
            // CodeObject::kwonly_defaults_mask / MAKE_FUNCTION).
            if code.kwonlyarg_count > 0 {
                // A live `__kwdefaults__` dict set on the function (either the
                // default one or a REPLACEMENT — `f.__kwdefaults__ = {...}`
                // must affect subsequent calls, test_keywordonlyarg's
                // testKwDefaults) is the source of truth for kwonly defaults,
                // overriding the compiled-in ones.
                let live_kwdefaults: Option<Box<crate::object::PyDict>> =
                    inner_f.dict.get("__kwdefaults__").and_then(|v| {
                        if let PyObject::Dict(d) = &*v.borrow() {
                            Some(d.clone())
                        } else {
                            None
                        }
                    });
                let kwonly_start = code.arg_count + if code.vararg_name.is_some() { 1 } else { 0 };
                // Build the name -> default map FIRST by consuming the
                // compiled-in defaults list sequentially over the FULL
                // kwonly parameter list. Applying per-slot while iterating
                // skipped explicitly-bound params WITHOUT consuming their
                // default, shifting every later default onto the wrong
                // parameter (observed: ConfigParser(interpolation=<bool>)).
                let name_to_default: std::collections::HashMap<String, PyObjectRef> =
                    match &live_kwdefaults {
                        Some(d) => d
                            .items()
                            .into_iter()
                            .filter_map(|(k, v)| Some((k.str(), v)))
                            .collect(),
                        None => {
                            let mut m = std::collections::HashMap::new();
                            let mut idx = code.num_defaults;
                            for (k, has_default) in code.kwonly_defaults_mask.iter().enumerate() {
                                let _ = k;
                                if *has_default {
                                    if let Some(v) = defaults.get(idx).cloned() {
                                        let name_str = interner::lookup_str(
                                            new_frame.code.varnames[kwonly_start + k],
                                        )
                                        .to_string();
                                        m.insert(name_str, v);
                                    }
                                    idx += 1;
                                }
                            }
                            m
                        }
                    };
                for k in 0..code.kwonly_defaults_mask.len() {
                    let idx = kwonly_start + k;
                    if idx >= new_frame.fast_locals.len()
                        || new_frame.fast_locals[idx].is_some()
                    {
                        continue;
                    }
                    let name_str =
                        interner::lookup_str(new_frame.code.varnames[idx]).to_string();
                    if let Some(val) = name_to_default.get(&name_str) {
                        new_frame.insert_local(&name_str, val.clone());
                        new_frame.fast_locals[idx] = Some(val.clone());
                    }
                }
            }

            // Any formal positional/keyword-only parameter still unbound at
            // this point has no value at all — real Python's `TypeError:
            // ...() missing N required positional/keyword-only argument(s):
            // '...'`, previously never checked.
            let missing_positional: Vec<String> = (0..named_params)
                .filter(|&i| i >= new_frame.fast_locals.len() || new_frame.fast_locals[i].is_none())
                .map(|i| interner::lookup_str(new_frame.code.varnames[i]).to_string())
                .collect();
            if !missing_positional.is_empty() {
                self.release_frame(new_frame);
                let n = missing_positional.len();
                return Err(PyError::type_error(format!(
                    "{}() missing {} required positional argument{}: {}",
                    fname,
                    n,
                    if n == 1 { "" } else { "s" },
                    format_missing_names(&missing_positional)
                )));
            }
            let missing_kwonly: Vec<String> = (kwonly_start..kwonly_start + code.kwonlyarg_count)
                .filter(|&i| i >= new_frame.fast_locals.len() || new_frame.fast_locals[i].is_none())
                .map(|i| interner::lookup_str(new_frame.code.varnames[i]).to_string())
                .collect();
            if !missing_kwonly.is_empty() {
                self.release_frame(new_frame);
                let n = missing_kwonly.len();
                return Err(PyError::type_error(format!(
                    "{}() missing {} required keyword-only argument{}: {}",
                    fname,
                    n,
                    if n == 1 { "" } else { "s" },
                    format_missing_names(&missing_kwonly)
                )));
            }

            self.push_frame(new_frame);
            let result = self.execute();
            if let Some(frame) = self.frames.pop() {
                self.release_frame(frame);
            }
            return result;
        })();
        Some(result)
    }

    pub(crate) fn handle_type_call(
        &mut self,
        callable: &PyObjectRef,
        args: Vec<PyObjectRef>,
        keywords: Vec<(String, PyObjectRef)>,
    ) -> Option<PyResult<PyObjectRef>> {
        if !matches!(&*callable.borrow(), PyObject::Type { .. }) { return None; }
        let result: PyResult<PyObjectRef> = (|| {
        let type_construct_info = if let PyObject::Type { dict, mro, .. } = &*callable.borrow() {
            let native_kind = dict
                .get_str(crate::object::NATIVE_BASE_MARKER)
                .map(|v| v.str());
            let init_func = dict.get_str("__init__").cloned().or_else(|| {
                for base in mro.iter().skip(1) {
                    if let PyObject::Type {
                        name: base_name,
                        dict: base_dict,
                        ..
                    } = &*base.borrow()
                    {
                        // Every class implicitly inherits from `object`,
                        // whose own __init__ is a universal no-op. For a
                        // class that also has a native base (e.g.
                        // `class SafeString(str, SafeData): ...`), that
                        // no-op would otherwise always be found first and
                        // preempt real native construction — skip it here
                        // so synthesize_native_init below gets a chance
                        // unless something more specific actually overrides
                        // __init__.
                        if native_kind.is_some() && base_name == "object" {
                            continue;
                        }
                        if let Some(val) = base_dict.get_str("__init__") {
                            return Some(val.clone());
                        }
                    }
                }
                None
            });
            Some((native_kind, init_func))
        } else {
            None
        };
        // The `callable.borrow()` above must be dropped (it already is, by
        // this point — the `if let` scrutinee's temporary ends with the
        // `if let` expression) before calling `__init__` below: `__init__`'s
        // body commonly references its own class by name (e.g. a
        // class-level counter like `Field.creation_counter += 1`, a
        // widespread real-world pattern, not specific to any one
        // library) — a STORE_ATTR on `callable` while this function still
        // held it borrowed here was a genuine double-borrow panic.
        if let Some((native_kind, init_func)) = type_construct_info {
            // A user-defined `__new__` (a Python Function, not the native
            // float/int/... `__new__`) must be called and its result
            // returned (class Foo3(float): def __new__(...): return
            // float.__new__(cls, 2*value) — Foo3(21) == 42). The native
            // __new__ on the base type builds the default instance.
            let custom_new = crate::object::lookup_dunder_via_mro(&callable, "__new__")
                .filter(|f| matches!(&*f.borrow(), PyObject::Function(_)));
            if let Some(new_fn) = custom_new {
                let mut new_args = args.clone();
                new_args.insert(0, callable.clone());
                let kw_clone = keywords.clone();
                let result = self.call_function(new_fn, new_args, kw_clone)?;
                // A user exception class whose `__new__` returns a
                // non-BaseException must raise TypeError (CPython: "calling
                // <class '...'> should have returned an instance of
                // BaseException, not <class 'list'>"). Without the check the
                // raw non-exception value would be raised/propagated and
                // escape every `except BaseException`.
                if crate::object::find_exception_base_name(&callable).is_some() {
                    let is_exc = match &*result.borrow() {
                        PyObject::Exception { .. } | PyObject::ExceptionGroup { .. } => true,
                        PyObject::Instance { typ, .. } => {
                            crate::object::find_exception_base_name(typ).is_some()
                        }
                        _ => false,
                    };
                    if !is_exc {
                        let result_typ = result.borrow().type_name();
                        return Err(PyError::type_error(format!(
                            "calling {} should have returned an instance of BaseException, not <class '{}'>",
                            callable.repr(),
                            result_typ
                        )));
                    }
                }
                // CPython: if __new__ returned an instance of this class,
                // AND __init__ is defined (and different from the base),
                // call __init__ before returning.
                // If __new__ returned an instance of this class AND __init__
                // is defined (and not the base object.__init__ no-op), call
                // __init__ — CPython's type_call always does this when
                // isinstance(result, cls) is true.
                let r = result.borrow();
                let is_instance_of_class = match &*r {
                    PyObject::Instance { typ, .. } => {
                        if typ.is(&callable) {
                            true
                        } else if let PyObject::Type { mro, .. } = &*typ.borrow() {
                            mro.iter().any(|b| b.is(&callable))
                        } else {
                            false
                        }
                    }
                    _ => false,
                };
                drop(r);
                if is_instance_of_class && init_func.is_some() {
                    let init_fn = init_func.clone().unwrap();
                    // Skip object.__init__ (universal no-op for native types)
                    let skip = matches!(&*init_fn.borrow(), PyObject::BuiltinFunction { name, .. } if name == "__init__");
                    if !skip {
                        let mut init_args = args.clone();
                        init_args.insert(0, result.clone());
                        self.call_function(init_fn, init_args, keywords.clone())?;
                    }
                }
                return Ok(result);
            }
            // ABC enforcement: if the class has __abstractmethods__ that is
            // non-empty, instantiation must raise TypeError (CPython:
            // "Can't instantiate abstract class ... with abstract methods").
            let abstracts_opt: Option<PyObjectRef> = (|| {
                match callable.borrow().get_attribute("__abstractmethods__") {
                    Ok(v) => Some(v),
                    Err(_) => None,
                }
            })();
            if let Some(abstracts) = abstracts_opt {
                let n = match &*abstracts.borrow() {
                    PyObject::FrozenSet(s) => s.len(),
                    PyObject::Set(s) => s.len(),
                    _ => 0,
                };
                if n > 0 {
                    // Collect the abstract method names for the error message.
                    let names: Vec<String> = match &*abstracts.borrow() {
                        PyObject::FrozenSet(s) => s.iter().map(|v| v.str()).collect(),
                        PyObject::Set(s) => s.iter().map(|v| v.str()).collect(),
                        _ => vec![],
                    };
                    let mut sorted = names;
                    sorted.sort();
                    return Err(PyError::type_error(format!(
                        "Can't instantiate abstract class {} with abstract method{} {}",
                        callable.borrow().type_name(),
                        if sorted.len() == 1 { "" } else { "s" },
                        sorted.join(", ")
                    )));
                }
            }

            let mut instance_dict = AttrMap::new();
            if let Some(kind) = &native_kind {
                instance_dict.insert(
                    crate::object::NATIVE_BACKING_KEY.to_string(),
                    crate::object::make_native_backing(kind),
                );
            }
            let instance = PyObjectRef::new(PyObject::Instance {
                typ: callable.clone(),
                dict: instance_dict,
            });
            // The native VALUE comes from `__new__(cls, *args)` — CPython
            // builds it BEFORE `__init__` runs, so even a custom `__init__`
            // (which overrides the native float/int/... init) must NOT leave
            // the backing at its default (`class Foo(float): def __init__
            // (self, x, ...): ...; Foo(2.5)` is still 2.5 — test_float's
            // test_keywords_in_subclass). Synthesize from the constructor
            // args unconditionally when there's a native base.
            if let Some(kind) = &native_kind {
                // A CONTAINER subclass (`class Counter(dict)`, `class
                // MyList(list)`) with a custom Python `__init__` is
                // different: `dict.__new__`/`list.__new__` ignore the
                // constructor args (the backing starts EMPTY) and the
                // custom `__init__` is what populates it (e.g.
                // `Counter('aabbc')` counts via its own `update`). Building
                // the backing from the args first (`builtin_dict('aabbc')`)
                // raises "cannot convert dictionary update sequence
                // element to a sequence" before `__init__` ever runs.
                let custom_py_init =
                    matches!(&init_func, Some(f) if matches!(&*f.borrow(), PyObject::Function(_)));
                let is_mutable_container = matches!(
                    kind.as_str(),
                    "dict" | "list" | "set" | "deque" | "bytearray"
                );
                let native = if custom_py_init && is_mutable_container {
                    crate::object::make_native_backing(kind)
                } else if custom_py_init
                    && matches!(
                        kind.as_str(),
                        "tuple" | "frozenset" | "bytes" | "str" | "int" | "float" | "complex"
                    )
                {
                    // Immutable base: its value is created by __new__, not
                    // __init__. When a subclass overrides __init__ with extra
                    // args (e.g. `class S(tuple): def __init__(self, arg,
                    // newarg=None)` → `S([1,2], newarg=3)`), those extra
                    // args belong to __init__, not to tuple.__new__. CPython's
                    // type_call slices them: __new__ receives only the
                    // iterable, __init__ receives the full args. Without this,
                    // passing extra kwargs to synthesize would either raise
                    // "tuple() takes no keyword arguments" incorrectly or,
                    // if skipped entirely, leave the backing empty (the
                    // observed [] vs [1,2] failure in
                    // test_keywords_in_subclass).
                    let truncated_args: &[PyObjectRef] =
                        if args.is_empty() { &[] } else { &args[0..1] };
                    crate::object::synthesize_native_init(kind, truncated_args, &[])?
                } else {
                    crate::object::synthesize_native_init(kind, &args, &keywords)?
                };
                if let PyObject::Instance { dict, .. } = &mut *instance.borrow_mut() {
                    dict.insert(crate::object::NATIVE_BACKING_KEY.to_string(), native);
                }
            } else if init_func.is_none()
                && crate::object::find_exception_base_name(&callable).is_some()
            {
                // `class MyError(Exception): pass` (no explicit __init__) —
                // real Python's `BaseException.__init__` always stores
                // `self.args = args`, which is what `str(exc)`/`repr(exc)`
                // and every uncaught-exception traceback print. Exception
                // builtins (Exception, ValueError, ...) are
                // `BuiltinFunction`s, not `PyObject::Type`s, so they never
                // appear in `mro` and were completely invisible to this
                // constructor logic — ANY user-defined exception subclass
                // (an extremely common, foundational pattern) silently got
                // no `args` at all, surfacing as "MyError: " (empty message)
                // or "Exception: re-raise" (the internal dispatch tag)
                // instead of the real message whenever it passed through a
                // `with`/`finally` or propagated uncaught.
                if let PyObject::Instance { dict, .. } = &mut *instance.borrow_mut() {
                    dict.insert_str("args", py_tuple(args.clone()));
                }
            }
            if let Some(init_func) = init_func {
                // Delegate to the real call_function instead of a hand-rolled
                // frame setup per callable kind — the latter (kept here for
                // a long time) never handled *args/**kwargs/default values at
                // all, silently binding missing parameters to None instead of
                // their real defaults and dropping every keyword argument
                // passed to the constructor. call_function already gets all
                // of that right for every callable variant (BuiltinFunction,
                // Function, Closure, ...).
                let mut init_args = vec![instance.clone()];
                init_args.extend(args);
                self.call_function(init_func, init_args, keywords)?;
            }
            return Ok(instance);
        }
            // Fallback: should never happen for a Type, but keep compiler happy
            Err(PyError::type_error("type construction failed"))
        })();
        Some(result)
    }

    pub(crate) fn handle_metaclass_call(
        &mut self,
        callable: &PyObjectRef,
        args: &[PyObjectRef],
        keywords: &[(String, PyObjectRef)],
    ) -> Option<PyResult<PyObjectRef>> {
        let looks_like_class_call = args.len() == 3
            && matches!(&*args[0].borrow(), PyObject::Str(_))
            && matches!(&*args[2].borrow(), PyObject::Dict(_));
        if !looks_like_class_call {
            return None;
        }
        let plain_type = self
            .builtins
            .get(&interner::intern("type"))
            .cloned();
        let callable_is_bare_type =
            plain_type.as_ref().map(|t| t.is(callable)).unwrap_or(false);
        let callable_is_metaclass = matches!(
            &*callable.borrow(),
            PyObject::Type { mro, .. } if plain_type
                .as_ref()
                .map(|t| mro.iter().any(|b| t.is(b)))
                .unwrap_or(false)
        );
        if !callable_is_bare_type && callable_is_metaclass {
            let mut new_args = vec![callable.clone()];
            new_args.extend(args.iter().cloned());
            if !keywords.is_empty() {
                let mut d = crate::object::PyDict::new();
                for (k, v) in keywords {
                    let _ = d.set(crate::object::py_str(k), v.clone());
                }
                new_args.push(crate::object::PyObjectRef::new(crate::object::PyObject::Dict(Box::new(d))));
            }
            return Some(self.type_new_impl(&new_args));
        }
        None
    }
}
