// Split out of the former monolithic object.rs (see the file-splitting
// refactor's memory entry for context) — this file holds the
// `__import__` builtin implementation.
use super::*;

// ---- __import__ builtin ----

/// True iff `name` is present in the `sys.modules` dict (the real import
/// cache). Used to distinguish "module already imported" from "module was
/// `del sys.modules['x']`'d and must be re-imported fresh".
fn sys_modules_has(vm: &crate::vm::VirtualMachine, name: &str) -> bool {
    if let Some(sys_mod) = vm.modules.get("sys") {
        if let PyObject::Module { dict, .. } = &*sys_mod.borrow() {
            if let Some(mod_dict) = dict.get_str("modules") {
                let md = mod_dict.borrow();
                if let PyObject::Dict(d) = &*md {
                    return d.get(&py_str(name)).ok().flatten().is_some();
                }
            }
        }
    }
    false
}

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
pub(crate) fn import_impl(
    vm: &mut crate::vm::VirtualMachine,
    name: &str,
    has_dots: bool,
    has_fromlist: bool,
) -> PyResult<PyObjectRef> {
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
                                mod_dict
                                    .borrow_mut()
                                    .set_attribute(&top_name, module.clone())
                                    .ok();
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
                            mod_dict
                                .borrow_mut()
                                .set_attribute(name, module.clone())
                                .ok();
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

    // Check if already loaded — `sys.modules` is the source of truth for
    // import caching (a module `del sys.modules['x']`'d must re-import as a
    // fresh object, test_atexit's test_atexit_instances).
    if let Some(module) = vm.import_cached_or_fresh(&resolved_name) {
        return Ok(module);
    }

    // Try to import the module from file
    match vm.import_module_from_file(&resolved_name) {
        Ok(module) => {
            vm.modules.insert(resolved_name.clone(), module.clone());
            // Also add to sys.modules
            if let Some(sys_mod) = vm.modules.get("sys") {
                if let PyObject::Module { dict, .. } = &*sys_mod.borrow() {
                    if let Some(mod_dict) = dict.get_str("modules") {
                        mod_dict
                            .borrow_mut()
                            .set_attribute(&resolved_name, module.clone())
                            .ok();
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
        return Err(PyError::type_error(
            "__import__() requires at least 1 argument (module name)",
        ));
    }
    // Mirrors the same check in `vm.rs`'s direct-dispatch fast path — `name`
    // must actually be a `str`, not silently coerced via `.str()`.
    if !matches!(&*args[0].borrow(), PyObject::Str(_)) {
        return Err(PyError::type_error(
            "__import__() argument 'name' must be str",
        ));
    }
    let name = args[0].str();
    // See the matching check in `vm.rs`'s direct-dispatch fast path for why
    // this is gated on `level == 0` (an empty name is the normal encoding
    // of a pure relative import when `level>0`).
    let level = args
        .last()
        .and_then(|last| {
            if let PyObject::Dict(d) = &*last.borrow() {
                d.get(&py_str("level")).ok().flatten()
            } else {
                None
            }
        })
        .or_else(|| args.get(4).cloned())
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    if name.is_empty() && level == 0 {
        return Err(PyError::value_error("Empty module name"));
    }
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
    let fromlist = fromlist_arg.and_then(|fl| match &*fl.borrow() {
        PyObject::List(items) => Some(items.clone()),
        PyObject::Tuple(items) => Some(items.iter().cloned().collect()),
        _ => None,
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
    let program = parser
        .parse_program()
        .map_err(|e| PyError::type_error(format!("eval parse error: {}", e)))?;
    let mut compiler = crate::compiler::Compiler::new();
    let code = compiler
        .compile(&program, "<eval>")
        .map_err(|e| PyError::type_error(format!("eval compile error: {}", e)))?;
    let code2 = code.clone();
    // Use current VM if available via VM_PTR so exec() shares modules, sys.path, etc.
    match with_vm_mut(|vm| vm.run(code)) {
        Ok(Ok(val)) => Ok(val),
        Ok(Err(e)) => Err(PyError::type_error(format!("eval error: {}", e))),
        Err(_) => {
            let mut new_vm = crate::vm::VirtualMachine::new();
            new_vm
                .run(code2)
                .map_err(|e| PyError::type_error(format!("eval error: {}", e)))
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
        })()
        .map_err(|e| PyError::type_error(format!("exec error: {}", e)))?,
    };
    let code2 = code.clone();
    // Use current VM if available via VM_PTR so exec() shares modules, sys.path, etc.
    match with_vm_mut(|vm| vm.run(code)) {
        Ok(Ok(ref _val)) => Ok(py_none()),
        Ok(Err(e)) => Err(PyError::type_error(format!("exec error: {}", e))),
        Err(_) => {
            let mut new_vm = crate::vm::VirtualMachine::new();
            new_vm
                .run(code2)
                .map_err(|e| PyError::type_error(format!("exec error: {}", e)))?;
            Ok(py_none())
        }
    }
}

/// PEP 263 source-encoding-cookie detection: real Python scans the first
/// TWO lines of a `bytes` source for a `# -*- coding: <name> -*-`-shaped
/// comment before falling back to UTF-8. Only meaningful for `bytes`/
/// `bytearray` source (a `str` source is already decoded, nothing to
/// detect) — needed so `compile()` can decode non-UTF-8 source bytes
/// (e.g. Latin-1) correctly instead of corrupting/mis-tokenizing them.
fn detect_pep263_encoding(bytes: &[u8]) -> Option<String> {
    for line in bytes.split(|&b| b == b'\n').take(2) {
        let line = String::from_utf8_lossy(line);
        let trimmed = line.trim_start();
        if !trimmed.starts_with('#') {
            continue;
        }
        if let Some(idx) = trimmed.find("coding").map(|i| i + "coding".len()) {
            let rest = trimmed[idx..].trim_start();
            if let Some(rest) = rest.strip_prefix(':').or_else(|| rest.strip_prefix('=')) {
                let name: String = rest
                    .trim_start()
                    .chars()
                    .take_while(|c| {
                        c.is_ascii_alphanumeric() || *c == '-' || *c == '_' || *c == '.'
                    })
                    .collect();
                if !name.is_empty() {
                    return Some(name);
                }
            }
        }
    }
    None
}

/// Decode raw source bytes (`compile()`'s `source` argument as `bytes`/
/// `bytearray`, or a source file read off disk by the import machinery)
/// using its own PEP 263 coding cookie if present, defaulting to UTF-8
/// otherwise (real CPython's own default). STRICT: a file/bytes blob that
/// isn't valid in the implied encoding is a `SyntaxError` (real CPython's
/// `(unicode error) 'utf-8' codec can't decode byte ...`), NOT silently
/// lossy-corrupted — `test_utf8source.py::test_badsyntax` imports a
/// latin-1-encoded, no-cookie source file and requires the resulting
/// `SyntaxError` message to contain `'utf-8'`.
pub(crate) fn decode_source_bytes(bytes: &[u8]) -> PyResult<String> {
    let encoding = detect_pep263_encoding(bytes).unwrap_or_else(|| "utf-8".to_string());
    let normalized = encoding.to_ascii_lowercase().replace('_', "-");
    // A UTF-8 BOM is valid (U+FEFF) but must be stripped so it doesn't end
    // up as a stray character tokenizing the source.
    let src: &[u8] = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    match normalized.as_str() {
        "latin-1" | "latin1" | "iso-8859-1" | "iso8859-1" | "l1" => {
            Ok(src.iter().map(|&b| b as char).collect())
        }
        _ => match std::str::from_utf8(src) {
            Ok(s) => Ok(s.to_string()),
            Err(e) => Err(PyError::syntax_error(format!(
                "(unicode error) 'utf-8' codec can't decode byte 0x{:x} in position {}: invalid start byte",
                src.get(e.valid_up_to()).copied().unwrap_or(0),
                e.valid_up_to()
            ))),
        },
    }
}

pub fn builtin_compile(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.len() < 3 {
        return Err(PyError::type_error(
            "compile() requires 3 arguments (source, filename, mode)",
        ));
    }
    // `bytes`/`bytearray` source must be decoded using its OWN encoding
    // (PEP 263 coding cookie, defaulting to UTF-8) — `.str()` on a `bytes`
    // object is its Python REPR (`"b'...'"`, quotes and escapes included),
    // not a decode, which silently fed garbage/mis-shaped source into the
    // parser instead of the real characters (confirmed via
    // `test_utf8source.py::test_latin1`: `compile()` on Latin-1-encoded
    // source containing `Ç` never even defined the variable it assigned).
    let source = match &*args[0].borrow() {
        PyObject::Bytes(b) | PyObject::ByteArray(b) => decode_source_bytes(b)?,
        _ => args[0].str(),
    };
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
    // Real CPython's "single" mode parses a STATEMENT (interactive input can
    // be `def f(): ...` or an expression); only "eval" is expression-only.
    // Treating "single" as an expression parse (the previous behavior) made
    // `compile("def f(...): pass", "<t>", "single")` raise a spurious
    // SyntaxError (test_keywordonlyarg::testSyntaxForManyArguments, which
    // compiles a 300-argument `def` in "single" mode).
    let program = if mode == "eval" {
        crate::parser::try_parse_as_expression(&source).map_err(|e| PyError::syntax_error(e))?
    } else {
        let mut parser = crate::parser::Parser::new(&source);
        parser
            .parse_program()
            .map_err(|e| PyError::syntax_error(e))?
    };
    let mut compiler = crate::compiler::Compiler::new();
    let code = compiler
        .compile(&program, &filename)
        .map_err(|e| PyError::syntax_error(e))?;
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
            let strict = kwargs
                .get(&py_str("strict"))
                .ok()
                .flatten()
                .map(|v| v.truthy())
                .unwrap_or(false);
            (&args[..args.len() - 1], strict)
        } else {
            (args, false)
        }
    };
    if iterables.is_empty() {
        return Ok(PyObjectRef::new(PyObject::ZipIterator {
            iterators: vec![],
        }));
    }
    let iters: Vec<PyObjectRef> = iterables
        .iter()
        .map(|a| builtin_iter(&[a.clone()]))
        .collect::<PyResult<Vec<_>>>()?;
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
                    let longer_at = (0..iters.len())
                        .find(|i| !stopped_indices.contains(i))
                        .unwrap();
                    return Err(PyError::value_error(format!(
                        "zip() argument {} is shorter than argument {}",
                        shorter_at + 1,
                        longer_at + 1,
                    )));
                }
                break;
            }
            rows.push(py_tuple(row));
        }
        return Ok(PyObjectRef::new(PyObject::ListIter {
            list: rows,
            index: 0,
        }));
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
            if let PyObject::BuiltinFunction { func: bf, .. } = &*f.borrow() {
                bf(&a)
            } else {
                unreachable!()
            }
        }
        1 => {
            if let PyObject::BuiltinMethod {
                func: bf,
                self_obj: s,
                ..
            } = &*f.borrow()
            {
                let mut all_args = vec![s.clone()];
                all_args.extend(a);
                bf(&all_args)
            } else {
                unreachable!()
            }
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
                    (
                        inner_f.code.clone(),
                        inner_f.globals.clone(),
                        inner_f.defaults.clone(),
                        inner_f.closure.clone(),
                        inner_f.code.name,
                    )
                } else {
                    unreachable!()
                }
            };
            {
                if std::env::var("RPY_DEBUG_IMPORT").is_ok() {
                    eprintln!(
                        "BUILTIN_CALL (disposable VM): fname={} code_name={} filename={}",
                        crate::interner::lookup_str(fname),
                        crate::interner::lookup_str(code.name),
                        code.filename
                    );
                }
                let npos = a.len();
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
                let mut frame = crate::vm::Frame::new(
                    code.clone(),
                    g.clone(),
                    std::rc::Rc::clone(&vm.builtins),
                    None,
                );
                frame.closure = Box::new(closure);
                for i in 0..npos.min(named_params) {
                    if i < code.varnames.len() {
                        frame.fast_locals[i] = Some(a[i].clone());
                        frame.insert_local(
                            crate::interner::lookup_str(code.varnames[i]),
                            a[i].clone(),
                        );
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
                            frame.insert_local(
                                crate::interner::lookup_str(code.varnames[i]),
                                defaults[default_idx].clone(),
                            );
                        }
                    }
                }
                if let Some(kwarg_name) = &code.kwarg_name {
                    if let Some(idx) = code
                        .varnames
                        .iter()
                        .position(|n| crate::interner::lookup_str(*n) == kwarg_name.as_str())
                    {
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
                if let PyObject::BoundMethod {
                    func: bf,
                    self_obj: s,
                    ..
                } = &*obj
                {
                    (bf.clone(), s.clone())
                } else {
                    return Err(PyError::type_error("not a bound method"));
                }
            };
            let mut all_args = vec![self_obj];
            let _a_len = a.len();
            all_args.extend(a);
            builtin_call(&bf, &all_args)
        }
        4 => {
            // The `type` class called as a plain value (`map(type, seq)`,
            // `key=type`, ...) must take the single-argument `type(x)` form
            // (return x's type), not construct an instance of `type` — same
            // special case the main VM's `call_function` already has.
            if matches!(&*f.borrow(), PyObject::Type { name, .. } if name == "type") && a.len() == 1
            {
                return crate::object::builtin_type_of(&a);
            }
            // A real native value type (`str`, `int`, `bool`, `list`, ...)
            // called through THIS disposable dispatcher — e.g. `map(str,
            // seq)`/`filter(bool, seq)`, which store the callable and
            // invoke it later via `builtin_call` rather than the main VM's
            // own `call_function` — needs the SAME `NATIVE_VALUE_CTOR_KEY`
            // priority check `call_function` already does (see its own doc
            // comment in `vm.rs`), checked BEFORE the generic "build an
            // empty Instance + call __init__" path below. Without this,
            // `map(str, [x])` silently built a broken, empty
            // `PyObject::Instance` (since `str`/`bool`/etc. have no REAL
            // `__init__` of the kind this generic path expects) instead of
            // dispatching to `builtin_str`/`builtin_bool`/etc. — confirmed
            // via direct repro: `list(map(str, [SomeCustomClass()]))`
            // printed the generic `<instance object at 0x...>` fallback
            // instead of calling `__str__`, even though `str(x)` directly
            // (main VM dispatch) worked fine. Real trigger: CPython's own
            // `test_robotparser.py`'s `RobotFileParser.__str__`, which does
            // `map(str, entries)` internally.
            let native_ctor = if let PyObject::Type { dict, .. } = &*f.borrow() {
                dict.get_str(crate::object::NATIVE_VALUE_CTOR_KEY).cloned()
            } else {
                None
            };
            if let Some(ctor) = native_ctor {
                return builtin_call(&ctor, &a);
            }
            if matches!(&*f.borrow(), PyObject::Type { .. }) {
                let instance = PyObjectRef::new(PyObject::Instance {
                    typ: f.clone(),
                    dict: AttrMap::new(),
                });
                if let Some(init) = lookup_dunder_via_mro(&f, "__init__") {
                    call_bound_method(init, instance.clone(), a)?;
                }
                Ok(instance)
            } else {
                unreachable!()
            }
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
                } else {
                    return Err(PyError::type_error("not a partial"));
                }
            };
            let mut all_args = partial_args.clone();
            all_args.extend(a);
            builtin_call(&func, &all_args)
        }
        _ => Err(PyError::type_error(format!(
            "'{}' object is not callable",
            type_name
        ))),
    }
}
