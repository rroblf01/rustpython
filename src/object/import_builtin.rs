// Split out of the former monolithic object.rs (see the file-splitting
// refactor's memory entry for context) — this file holds the
// `__import__` builtin implementation.
use super::*;

// ---- __import__ builtin ----

// Extracted out of `builtin_import` so `vm.rs`'s `call_function` can invoke
// it directly with the real, live `&mut VirtualMachine` instead of going
// through `with_vm_mut` — `__import__()` is what every `import` STATEMENT
// desugars to at the bytecode level in real CPython, and while this
// interpreter's own `IMPORT_NAME` opcode handling doesn't call through this
// function, plenty of real code (`importlib.import_module`-adjacent
// patterns, direct `__import__("x")` calls) does invoke it explicitly —
// confirmed segfaulting via the simplest possible repro (`__import__("os")`
// at plain top level), the same unconditional `with_vm_mut`-aliasing UB
// found repeatedly elsewhere this session.
pub(crate) fn import_impl(vm: &mut crate::vm::VirtualMachine, name: &str, has_dots: bool, has_fromlist: bool) -> PyResult<PyObjectRef> {
    // With a non-empty fromlist and a dotted name, import the full module chain
    // and return the rightmost module. CPython behavior:
    //   __import__("certifi.core", ..., ["where"], 0)  -> imports certifi.core, returns certifi.core
    //   __import__("certifi.core", ..., [], 0)          -> imports certifi, returns certifi
    if has_dots && has_fromlist {
        // First, ensure the top-level package is imported (import_module_from_file
        // needs the parent in modules to resolve dotted names)
        let top_name = name.split('.').next().unwrap_or(name).to_string();
        if !vm.modules.contains_key(&top_name) {
            match vm.import_module_from_file(&top_name) {
                Ok(module) => {
                    vm.modules.insert(top_name.clone(), module.clone());
                    if let Some(sys_mod) = vm.modules.get("sys") {
                        if let PyObject::Module { dict, .. } = &*sys_mod.borrow() {
                            if let Some(mod_dict) = dict.get_str("modules") {
                                mod_dict.borrow_mut().set_attribute(&top_name, module.clone()).ok();
                            }
                        }
                    }
                }
                Err(e) => return Err(e),
            }
        }

        // Now import the full chain - import_module_from_file handles dotted
        // names when the parent is already in modules
        if let Some(module) = vm.modules.get(name) {
            return Ok(module.clone());
        }
        return match vm.import_module_from_file(name) {
            Ok(module) => {
                vm.modules.insert(name.to_string(), module.clone());
                if let Some(sys_mod) = vm.modules.get("sys") {
                    if let PyObject::Module { dict, .. } = &*sys_mod.borrow() {
                        if let Some(mod_dict) = dict.get_str("modules") {
                            mod_dict.borrow_mut().set_attribute(name, module.clone()).ok();
                        }
                    }
                }
                Ok(module)
            }
            Err(e) => Err(e),
        };
    }

    // Without fromlist (or non-dotted name), import only the top-level package
    let resolved_name = if has_dots {
        name.split('.').next().unwrap_or(name).to_string()
    } else {
        name.to_string()
    };

    // Check if already loaded
    if let Some(module) = vm.modules.get(&resolved_name) {
        return Ok(module.clone());
    }

    // Try to import the module from file
    match vm.import_module_from_file(&resolved_name) {
        Ok(module) => {
            vm.modules.insert(resolved_name.clone(), module.clone());
            // Also add to sys.modules
            if let Some(sys_mod) = vm.modules.get("sys") {
                if let PyObject::Module { dict, .. } = &*sys_mod.borrow() {
                    if let Some(mod_dict) = dict.get_str("modules") {
                        mod_dict.borrow_mut().set_attribute(&resolved_name, module.clone()).ok();
                    }
                }
            }
            Ok(module)
        }
        Err(e) => Err(e),
    }
}

pub fn builtin_import(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error("__import__() requires at least 1 argument (module name)"));
    }
    let name = args[0].str();
    // Handle fromlist: if provided, return the rightmost submodule. Real
    // code overwhelmingly calls `__import__(name, fromlist=[...])` with
    // `fromlist` as a KEYWORD argument (real trigger: CPython's own
    // `dbm/__init__.py`, `__import__(modname, fromlist=['open'])`), which
    // under this project's own calling convention arrives as a trailing
    // packed kwargs dict, NOT as a 4th positional argument — checking only
    // `args[3]` (matching real CPython's positional `__import__(name,
    // globals, locals, fromlist, level)` signature exactly) silently
    // missed the overwhelmingly common keyword form entirely, always
    // returning the top-level package instead of the requested submodule
    // and causing `mod.open` to resolve back to the PACKAGE's own `open`
    // (an infinite-recursion trap for `dbm`'s own dispatcher, which calls
    // `mod.open(...)` expecting `mod` to be the specific submodule).
    let kwargs_fromlist = args.last().and_then(|last| {
        if let PyObject::Dict(d) = &*last.borrow() {
            d.get(&py_str("fromlist")).ok().flatten()
        } else {
            None
        }
    });
    let fromlist_arg = kwargs_fromlist.or_else(|| args.get(3).cloned());
    let fromlist = fromlist_arg.and_then(|fl| {
        match &*fl.borrow() {
            PyObject::List(items) => Some(items.clone()),
            PyObject::Tuple(items) => Some(items.iter().cloned().collect()),
            _ => None,
        }
    });
    let has_dots = name.contains('.');
    let has_fromlist = fromlist.as_ref().map_or(false, |fl| !fl.is_empty());

    let import_result = with_vm_mut(|vm| -> PyResult<PyObjectRef> {
        import_impl(vm, &name, has_dots, has_fromlist)
    });

    match import_result {
        Ok(inner) => inner,
        Err(_) => Err(PyError::runtime_error("__import__: no active VM")),
    }
}

pub fn builtin_eval(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error("eval() requires at least 1 argument"));
    }
    let source = args[0].str();
    let mut parser = crate::parser::Parser::new(&source);
    let program = parser.parse_program().map_err(|e| PyError::type_error(format!("eval parse error: {}", e)))?;
    let mut compiler = crate::compiler::Compiler::new();
    let code = compiler.compile(&program, "<eval>").map_err(|e| PyError::type_error(format!("eval compile error: {}", e)))?;
    let code2 = code.clone();
    // Use current VM if available via VM_PTR so exec() shares modules, sys.path, etc.
    match with_vm_mut(|vm| vm.run(code)) {
        Ok(Ok(val)) => Ok(val),
        Ok(Err(e)) => Err(PyError::type_error(format!("eval error: {}", e))),
        Err(_) => {
            let mut new_vm = crate::vm::VirtualMachine::new();
            new_vm.run(code2).map_err(|e| PyError::type_error(format!("eval error: {}", e)))
        }
    }
}

pub fn builtin_exec(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error("exec() requires at least 1 argument"));
    }
    // Check if first arg is a code object (compile() result)
    let code = match &*args[0].borrow() {
        PyObject::Code(c) => (**c).clone(),
        _ => (|| -> Result<CodeObject, String> {
                let source = args[0].str();
                let mut parser = crate::parser::Parser::new(&source);
                let program = parser.parse_program()?;
                let mut compiler = crate::compiler::Compiler::new();
                compiler.compile(&program, "<exec>")
            })().map_err(|e| PyError::type_error(format!("exec error: {}", e)))?,
    };
    let code2 = code.clone();
    // Use current VM if available via VM_PTR so exec() shares modules, sys.path, etc.
    match with_vm_mut(|vm| vm.run(code)) {
        Ok(Ok(ref _val)) => Ok(py_none()),
        Ok(Err(e)) => Err(PyError::type_error(format!("exec error: {}", e))),
        Err(_) => {
            let mut new_vm = crate::vm::VirtualMachine::new();
            new_vm.run(code2).map_err(|e| PyError::type_error(format!("exec error: {}", e)))?;
            Ok(py_none())
        }
    }
}

pub fn builtin_compile(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 3 {
        return Err(PyError::type_error("compile() requires 3 arguments (source, filename, mode)"));
    }
    let source = args[0].str();
    let filename = args[1].str();
    let mode = args[2].str();
    // The `mode` argument was previously READ but never actually consulted
    // (`_mode`, underscore-prefixed — deliberately unused) — every call
    // parsed as a plain statement/module body regardless of "eval"/"exec"/
    // "single", so `compile(src, f, "eval")` produced a MODULE-shaped code
    // object (ending in `LOAD_CONST None; RETURN_VALUE`, discarding
    // whatever the expression computed) instead of an EXPRESSION-shaped one
    // (`RETURN_VALUE` with the actual computed value). Confirmed via the
    // simplest repro: `eval(compile("1+1", "<x>", "eval"))` returned `None`
    // instead of `2`, even though `eval("1+1")` (compiling from a raw
    // string, which goes through a SEPARATE, already-correct code path in
    // `vm.rs`'s own `exec`/`eval` special-casing) worked fine — the bug was
    // specifically in the `compile()` builtin, not `eval()` itself. "single"
    // (REPL/interactive mode) is treated the same as "eval" here — this
    // interpreter has no separate auto-print-via-displayhook mechanism for
    // it either way, so an expression's VALUE is at least preserved instead
    // of silently discarded, which is what actually mattered for real
    // callers (real trigger: a `doctest`-style engine using
    // `compile(example, "<doctest>", "eval")` to both execute an example
    // and recover its result for auto-printing).
    let program = if mode == "eval" || mode == "single" {
        crate::parser::try_parse_as_expression(&source).map_err(|e| PyError::syntax_error(e))?
    } else {
        let mut parser = crate::parser::Parser::new(&source);
        parser.parse_program().map_err(|e| PyError::syntax_error(e))?
    };
    let mut compiler = crate::compiler::Compiler::new();
    let code = compiler.compile(&program, &filename).map_err(|e| PyError::syntax_error(e))?;
    Ok(PyObjectRef::new(PyObject::Code(Rc::new(code))))
}

pub fn builtin_super(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    // super() with no args or super(class, instance)
    if args.len() == 2 {
        let cls = args[0].clone();
        let obj = args[1].clone();
        Ok(PyObjectRef::new(PyObject::Super { cls, obj }))
    } else {
        Err(PyError::type_error("super() requires 2 arguments"))
    }
}

pub fn builtin_map(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 2 {
        return Err(PyError::type_error("map() requires at least 2 arguments"));
    }
    let func = args[0].clone();
    let iter = builtin_iter(&[args[1].clone()])?;
    Ok(PyObjectRef::new(PyObject::MapIterator {
        func,
        iterator: Box::new(iter),
    }))
}

pub fn builtin_filter(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() != 2 {
        return Err(PyError::type_error("filter() requires exactly 2 arguments"));
    }
    let func = args[0].clone();
    let iter = builtin_iter(&[args[1].clone()])?;
    Ok(PyObjectRef::new(PyObject::FilterIterator {
        func,
        iterator: Box::new(iter),
    }))
}

pub fn builtin_zip(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error("zip() requires at least 1 argument"));
    }
    // Keyword args (only `strict` is defined for zip()) arrive packed into a
    // trailing dict, per the calling convention call_function uses for all
    // BuiltinFunction calls. Without stripping it here, `zip(a, b,
    // strict=True)` treated the kwargs dict itself as one more iterable to
    // zip — iterating a dict yields its keys, so it silently zipped in the
    // literal string "strict" as a bogus extra column instead of enforcing
    // equal lengths.
    let (iterables, strict) = {
        let last = args.last().unwrap();
        let last_borrowed = last.borrow();
        if let PyObject::Dict(kwargs) = &*last_borrowed {
            let strict = kwargs.get(&py_str("strict")).ok().flatten().map(|v| v.truthy()).unwrap_or(false);
            (&args[..args.len() - 1], strict)
        } else {
            (args, false)
        }
    };
    if iterables.is_empty() {
        return Ok(PyObjectRef::new(PyObject::ZipIterator { iterators: vec![] }));
    }
    let iters: Vec<PyObjectRef> = iterables.iter().map(|a| builtin_iter(&[a.clone()])).collect::<PyResult<Vec<_>>>()?;
    if strict {
        // Eagerly materialize and check equal lengths — the lazy
        // ZipIterator has no way to distinguish "ran out because lengths
        // differ" from "ran out because we're done" once iteration starts,
        // so `strict` must be enforced up front.
        let mut rows: Vec<PyObjectRef> = Vec::new();
        loop {
            let mut row = Vec::with_capacity(iters.len());
            let mut stopped_indices = Vec::new();
            for (idx, it) in iters.iter().enumerate() {
                match builtin_next(&[it.clone()]) {
                    Ok(v) => row.push(v),
                    Err(e) if is_stop_iteration_error(&e) => stopped_indices.push(idx),
                    Err(e) => return Err(e),
                }
            }
            if !stopped_indices.is_empty() {
                if stopped_indices.len() != iters.len() {
                    let shorter_at = stopped_indices[0];
                    let longer_at = (0..iters.len()).find(|i| !stopped_indices.contains(i)).unwrap();
                    return Err(PyError::value_error(format!(
                        "zip() argument {} is shorter than argument {}",
                        shorter_at + 1, longer_at + 1,
                    )));
                }
                break;
            }
            rows.push(py_tuple(row));
        }
        return Ok(PyObjectRef::new(PyObject::ListIter { list: rows, index: 0 }));
    }
    Ok(PyObjectRef::new(PyObject::ZipIterator { iterators: iters }))
}

pub fn builtin_call(func: &PyObjectRef, args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let f = func.clone();
    let a = args.to_vec();
    let type_name = f.get_type_name();
    let kind = {
        let obj = f.borrow();
        match &*obj {
            PyObject::BuiltinFunction { .. } => 0,
            PyObject::BuiltinMethod { .. } => 1,
            PyObject::Function(_) => 2,
            PyObject::BoundMethod { .. } => 3,
            PyObject::Type { .. } => 4,
            PyObject::BuildClass => 5,
            PyObject::Partial { .. } => 6,
            _ => 7,
        }
    };
    match kind {
        0 => {
            if let PyObject::BuiltinFunction { func: bf, .. } = &*f.borrow() { bf(&a) } else { unreachable!() }
        }
        1 => {
            if let PyObject::BuiltinMethod { func: bf, self_obj: s, .. } = &*f.borrow() {
                let mut all_args = vec![s.clone()];
                all_args.extend(a);
                bf(&all_args)
            } else { unreachable!() }
        }
        2 => {
            // See `NativeDispatchRecursionGuard`'s own doc comment (`core.rs`)
            // — without this, recursion flowing through this disposable-VM
            // dispatch path overflows the real native stack instead of
            // raising a catchable `RecursionError`, since each nested call
            // resets its own fresh VM's frame counter to zero.
            let _guard = crate::object::NativeDispatchRecursionGuard::enter()?;
            // Clone everything needed out under a SHORT borrow and drop it
            // immediately — the previous version held `f.borrow()` across
            // the ENTIRE disposable-VM `vm.execute()` call below, which
            // runs arbitrary Python (the function's own body). Any function
            // that sets an attribute on ITSELF during its own execution
            // (`func.some_attr = ...` — a real, deliberately adversarial
            // CPython regression test: `test_iter.py`'s
            // `test_iter_function_concealing_reentrant_exhaustion`,
            // gh-101892, whose `spam()` does exactly this) hit `STORE_ATTR`
            // trying to `borrow_mut()` the SAME `PyObjectRef` this borrow
            // was still holding, panicking the whole process with "RefCell
            // already borrowed" instead of just running the (perfectly
            // ordinary) attribute assignment.
            let (code, g, defaults, closure, fname) = {
                let obj = f.borrow();
                if let PyObject::Function(inner_f) = &*obj {
                    (inner_f.code.clone(), inner_f.globals.clone(), inner_f.defaults.clone(), inner_f.closure.clone(), inner_f.code.name)
                } else { unreachable!() }
            };
            {
                if std::env::var("RPY_DEBUG_IMPORT").is_ok() {
                    eprintln!("BUILTIN_CALL (disposable VM): fname={} code_name={} filename={}", crate::interner::lookup_str(fname), crate::interner::lookup_str(code.name), code.filename);
                }
                let npos = a.len();
                let named_params = if code.vararg_name.is_some() || code.kwarg_name.is_some() {
                    code.varnames.iter().position(|n| {
                        code.vararg_name.as_ref().map(|b| b.as_str()) == Some(crate::interner::lookup_str(*n)) || code.kwarg_name.as_ref().map(|b| b.as_str()) == Some(crate::interner::lookup_str(*n))
                    }).unwrap_or(code.varnames.len())
                } else {
                    code.varnames.len()
                };
                // See the matching fix (and its full explanation) in
                // `call_bound_method`'s own `PyObject::Function` arm just
                // above — this is a second, independent implementation of
                // the exact same "call a Function via a disposable VM"
                // pattern, with the exact same two bugs: the callee frame's
                // `closure` was never set (breaking any closure-capturing
                // function invoked through `filter()`/`map()`/etc., e.g. a
                // nested helper closing over an enclosing method's `self` —
                // real trigger: CPython 3.14's own `unittest/loader.py`'s
                // `getTestCaseNames`), and the frame's `builtins` map used
                // to come from a second, independent `create_builtins()`
                // call instead of the disposable VM's own, breaking
                // pointer-identity checks like the `type(x)` special case.
                let mut vm = crate::vm::VirtualMachine::new();
                let mut frame = crate::vm::Frame::new(code.clone(), g.clone(), std::rc::Rc::clone(&vm.builtins), None);
                frame.closure = Box::new(closure);
                for i in 0..npos.min(named_params) {
                    if i < code.varnames.len() {
                        frame.fast_locals[i] = Some(a[i].clone());
                        frame.insert_local(crate::interner::lookup_str(code.varnames[i]), a[i].clone());
                    }
                }
                if let Some(vararg_name) = &code.vararg_name {
                    let mut extra = Vec::new();
                    for i in named_params..npos {
                        extra.push(a[i].clone());
                    }
                    let vararg_val = py_tuple(extra);
                    // Must ALSO land in `fast_locals` — same missing-write
                    // bug, same fix, as the analogous vararg-packing block
                    // in `call_bound_method` just below in this same file
                    // (real trigger: a `*args`-taking plain function invoked
                    // via `map()`/`filter()`/etc. through THIS disposable-VM
                    // path, not just the bound-method-via-class-construction
                    // case that surfaced it).
                    if let Some(idx) = code.varnames.iter().position(|n| crate::interner::lookup_str(*n) == vararg_name.as_str()) {
                        if idx < frame.fast_locals.len() {
                            frame.fast_locals[idx] = Some(vararg_val.clone());
                        }
                    }
                    frame.insert_local(vararg_name.as_str(), vararg_val);
                }
                if npos < named_params {
                    let num_defaults = code.num_defaults;
                    for i in npos..named_params {
                        let default_idx = num_defaults.saturating_sub(named_params - i);
                        if default_idx < defaults.len() {
                            // Must also land in `fast_locals` — LOAD_FAST
                            // reads that, not the `insert_local` name dict.
                            // Missing this meant any defaulted parameter
                            // left unfilled by a call through this disposable-
                            // VM path (e.g. a plain function invoked via
                            // `map()`/`filter()` with fewer positional args
                            // than it declares) raised "local variable
                            // referenced before assignment" the moment the
                            // function body read it — real trigger:
                            // `unittest`'s own `_common_shorten_repr`,
                            // `tuple(map(safe_repr, args))` calling
                            // `safe_repr(obj, short=False)` with just one arg.
                            if i < frame.fast_locals.len() {
                                frame.fast_locals[i] = Some(defaults[default_idx].clone());
                            }
                            frame.insert_local(crate::interner::lookup_str(code.varnames[i]), defaults[default_idx].clone());
                        }
                    }
                }
                if let Some(kwarg_name) = &code.kwarg_name {
                    if let Some(idx) = code.varnames.iter().position(|n| crate::interner::lookup_str(*n) == kwarg_name.as_str()) {
                        if idx < frame.fast_locals.len() && frame.fast_locals[idx].is_none() {
                            frame.fast_locals[idx] = Some(py_dict());
                        }
                    }
                    if !frame.contains_local(kwarg_name) {
                        frame.insert_local(kwarg_name.as_str(), py_dict());
                    }
                }
                vm.frames.push(frame);
                vm.execute()
            }
        }
        3 => {
            let (bf, self_obj) = {
                let obj = f.borrow();
                if let PyObject::BoundMethod { func: bf, self_obj: s, .. } = &*obj {
                    (bf.clone(), s.clone())
                } else { return Err(PyError::type_error("not a bound method")); }
            };
            let mut all_args = vec![self_obj];
            let _a_len = a.len();
            all_args.extend(a);
            builtin_call(&bf, &all_args)
        }
        4 => {
            if matches!(&*f.borrow(), PyObject::Type { .. }) {
                let instance = PyObjectRef::new(PyObject::Instance {
                    typ: f.clone(),
                    dict: AttrMap::new(),
                });
                if let Some(init) = lookup_dunder_via_mro(&f, "__init__") {
                    call_bound_method(init, instance.clone(), a)?;
                }
                Ok(instance)
            } else { unreachable!() }
        }
        5 => {
            let instance = PyObjectRef::new(PyObject::Instance {
                typ: f.clone(),
                dict: AttrMap::new(),
            });
            Ok(instance)
        }
        6 => {
            let (func, partial_args) = {
                let obj = f.borrow();
                if let PyObject::Partial { func: bf, args: pa } = &*obj {
                    (bf.clone(), pa.clone())
                } else { return Err(PyError::type_error("not a partial")); }
            };
            let mut all_args = partial_args.clone();
            all_args.extend(a);
            builtin_call(&func, &all_args)
        }
        _ => Err(PyError::type_error(format!("'{}' object is not callable", type_name))),
    }
}

