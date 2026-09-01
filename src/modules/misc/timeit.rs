use crate::object::*;
use std::collections::HashMap;
use std::cell::RefCell;
use std::rc::Rc;
use num_bigint::BigInt;
use num_traits::ToPrimitive;

/// Compile `stmt` once and run it `number` times in pooled VMs.
/// Returns elapsed total seconds.
fn timeit_run_compiled(code: &crate::bytecode::CodeObject, number: u64) -> PyResult<f64> {
    let start = std::time::Instant::now();
    for _ in 0..number {
        let mut vm = crate::vm::VirtualMachine::take_disposable();
        let r = vm.run(code.clone());
        crate::vm::VirtualMachine::release_disposable(vm);
        r.map_err(|e| PyError::type_error(format!("timeit error: {}", e)))?;
    }
    Ok(start.elapsed().as_secs_f64())
}

fn timeit_compile_src(src: &str, what: &str) -> PyResult<crate::bytecode::CodeObject> {
    let mut parser = crate::parser::Parser::new(src);
    let program = parser
        .parse_program()
        .map_err(|e| PyError::type_error(format!("timeit {} parse error: {}", what, e)))?;
    let mut compiler = crate::compiler::Compiler::new();
    compiler
        .compile(&program, "<timeit>")
        .map_err(|e| PyError::type_error(format!("timeit {} compile error: {}", what, e)))
}
fn timeit_native_compile(src: &str) -> PyResult<PyObjectRef> {
    let code = timeit_compile_src(src, "compile")?;
    Ok(PyObjectRef::imm(PyObject::Code(Rc::new(code))))
}

fn timeit_native_run_in_globals(code_obj: &PyObjectRef, globals: &PyObjectRef) -> PyResult<PyObjectRef> {
    let code_rc = match &*code_obj.borrow() {
        PyObject::Code(c) => c.clone(),
        _ => return Err(PyError::type_error("_run_in_globals expects a code object")),
    };
    let mut map: HashMap<crate::interner::StrId, PyObjectRef> = HashMap::new();
    if let PyObject::Dict(d) = &*globals.borrow() {
        for (k, v) in d.items() {
            if let PyObject::Str(sk) = &*k.borrow() {
                map.insert(crate::interner::intern(sk.as_str()), v.clone());
            }
        }
    }
    let bmod = crate::vm::get_shared_builtins_module();
    map.insert(crate::interner::intern("__builtins__"), bmod);
    // Inside this pooled-VM execution, sys.modules is the shared truth:
    // `import timeit` must resolve to the REAL module object (with
    // test-injected attributes like _fake_timer), not a stale snapshot.
    crate::vm::set_sys_modules_priority(true);
    let mut vm = crate::vm::VirtualMachine::take_disposable();
    vm.globals = Rc::new(RefCell::new(map));
    let r = vm.run((*code_rc).clone());
    crate::vm::set_sys_modules_priority(false);
    crate::vm::VirtualMachine::release_disposable(vm);
    r
}


/// Native `timeit.Timer`.
///
/// Faithful enough for CPython's own `test_timeit.py`:
/// * `stmt`/`setup` may be strings (compiled once, executed in the given
///   or synthesized globals) OR callables (invoked directly).
/// * `timer` must be a callable used as the clock — the returned "elapsed"
///   is `timer_end - timer_start`, which is how the fake-timer tests get
///   exact deltas (`delta_time == number`).
/// * `globals` is the namespace statements execute in.
fn split_kwargs(args: &[PyObjectRef]) -> (usize, Vec<(String, PyObjectRef)>) {
    if let Some(last) = args.last() {
        let b = last.borrow();
        if let PyObject::Dict(d) = &*b {
            if args.len() >= 2 {
                let pairs = d.items();
                let kw: Vec<(String, PyObjectRef)> = pairs
                    .iter()
                    .map(|(k, v)| (k.str(), v.clone()))
                    .collect();
                return (args.len() - 1, kw);
            }
        }
    }
    (args.len(), Vec::new())
}

fn kw_lookup<'a>(kw: &'a [(String, PyObjectRef)], name: &str) -> Option<&'a PyObjectRef> {
    kw.iter().find(|(k, _)| k == name).map(|(_, v)| v)
}

fn make_timeit_type() -> PyObjectRef {
    let mut type_dict: HashMap<String, PyObjectRef> = HashMap::new();

    macro_rules! t_method {
        ($name:expr, $func:expr) => {
            type_dict.insert(
                $name.to_string(),
                PyObjectRef::imm(PyObject::BuiltinFunction {
                    name: $name.to_string(),
                    func: $func,
                }),
            );
        };
    }

    // helper: call a Python callable from native context
    fn py_call(f: PyObjectRef, args: Vec<PyObjectRef>) -> PyResult<PyObjectRef> {
        if let PyObject::Instance { typ, .. } = &*f.borrow() {
            if let Some(cm) = crate::object::lookup_dunder_via_mro(typ, "__call__") {
                return crate::object::call_bound_method(cm, f.clone(), args);
            }
            return Err(PyError::type_error("object is not callable"));
        }
        // Python functions need a VM; use the disposable-VM caller.
        crate::object::call_function_disposable(&f, args, vec![])
    }

    t_method!("__init__", |args| {
        if std::env::var("RPY_DBG_TT").is_ok() {
            eprintln!("NATIVE Timer.__init__ nargs={} a1={:?}", args.len(), args.get(1).map(|v| v.str()));
        }
        let self_obj = args
            .first()
            .cloned()
            .ok_or_else(|| PyError::type_error("__init__ missing self"))?;
        let (n, kw) = split_kwargs(args);
        let getp = |i: usize| -> Option<PyObjectRef> { args.get(i + 1).cloned() };
        let pos_stmt = getp(0);
        let pos_setup = getp(1);
        let pos_timer = getp(2);
        let stmt = kw_lookup(&kw, "stmt").or(pos_stmt.as_ref()).cloned();
        let setup = kw_lookup(&kw, "setup").or(pos_setup.as_ref()).cloned();
        let timer = kw_lookup(&kw, "timer").or(pos_timer.as_ref()).cloned();
        let globals_v = kw_lookup(&kw, "globals").cloned();
        {
            let mut b = self_obj.borrow_mut();
            if let PyObject::Instance { dict, .. } = &mut *b {
                dict.insert_str("_stmt", stmt.clone().unwrap_or_else(|| py_str("pass")));
                dict.insert_str("_setup", setup.clone().unwrap_or_else(|| py_str("pass")));
                dict.insert_str(
                    "_timer",
                    timer.unwrap_or_else(|| py_none()),
                );
                dict.insert_str(
                    "_globals",
                    globals_v.unwrap_or_else(|| py_none()),
                );
            }
        }
        Ok(py_none())
    });

    // Runs one timed measurement. Returns elapsed seconds per CPython rules:
    // uses the injected timer when present.
    fn run_timed(
        self_obj: &PyObjectRef,
        number: u64,
    ) -> PyResult<f64> {
        let (stmt_v, setup_v, timer_v, globals_v) = {
            let b = self_obj.borrow();
            let get = |k: &str| -> Option<PyObjectRef> {
                if let PyObject::Instance { dict, .. } = &*b {
                    dict.get_str(k).cloned()
                } else {
                    None
                }
            };
            (get("_stmt"), get("_setup"), get("_timer"), get("_globals"))
        };

        let is_callable = |v: &Option<PyObjectRef>| -> bool {
            v.as_ref()
                .map(|x| {
                    matches!(
                        &*x.borrow(),
                        PyObject::Function(_)
                            | PyObject::BuiltinFunction { .. }
                            | PyObject::BuiltinMethod { .. }
                            | PyObject::BoundMethod { .. }
                            | PyObject::Instance { .. }
                    )
                })
                .unwrap_or(false)
        };

        // Prepare globals dict (PyObject::Dict) for string execution.
        let globals_dict: PyObjectRef = match globals_v {
            Some(g) if !matches!(&*g.borrow(), PyObject::None) => g,
            _ => PyObjectRef::imm(PyObject::Dict(Box::new(PyDict::new()))),
        };

        // Resolve setup: compile or wrap callable
        enum Prepared {
            Src(std::rc::Rc<crate::bytecode::CodeObject>),
            Callable(PyObjectRef),
        }
        let setup_prep: Option<Prepared> = match &setup_v {
            Some(v) if is_callable(&Some(v.clone())) => Some(Prepared::Callable(v.clone())),
            Some(v) => {
                let src = v.str();
                if src.trim().is_empty() || src.trim() == "pass" {
                    None
                } else {
                    let cobj = timeit_native_compile(&src)?;
                    let c = match &*cobj.borrow() {
                        PyObject::Code(c) => c.clone(),
                        _ => unreachable!(),
                    };
                    Some(Prepared::Src(c))
                }
            }
            None => None,
        };
        let stmt_prep = match &stmt_v {
            Some(v) if is_callable(&Some(v.clone())) => Prepared::Callable(v.clone()),
            Some(v) => {
                let src = v.str();
                Prepared::Src(match timeit_native_compile(&src)? {
                    PyObjectRef::Imm(rc) => match &*rc.borrow() {
                        PyObject::Code(c) => c.clone(),
                        _ => unreachable!(),
                    },
                    _ => unreachable!(),
                })
            }
            None => return Err(PyError::type_error("timeit missing stmt")),
        };

        // Run setup once (not timed)
        match &setup_prep {
            Some(Prepared::Callable(f)) => {
                py_call(f.clone(), vec![])?;
            }
            Some(Prepared::Src(code)) => {
                let cobj = PyObjectRef::imm(PyObject::Code(code.clone()));
                timeit_native_run_in_globals(&cobj, &globals_dict)?;
            }
            None => {}
        }

        // Clock
        use std::time::Instant;
        let timer_is_usable = timer_v.as_ref().map(|t| {
            match &*t.borrow() {
                PyObject::None => false,
                PyObject::Instance { typ, .. } => {
                    crate::object::lookup_dunder_via_mro(typ, "__call__").is_some()
                }
                _ => true,
            }
        }).unwrap_or(false);
        let has_py_timer = timer_is_usable;

        if has_py_timer {
            let timer = timer_v.clone().unwrap();
            let t0 = py_call(timer.clone(), vec![])?;
            match &stmt_prep {
                Prepared::Callable(f) => {
                    for _ in 0..number {
                        py_call(f.clone(), vec![])?;
                    }
                }
                Prepared::Src(code) => {
                    let cobj = PyObjectRef::imm(PyObject::Code(code.clone()));
                    for _ in 0..number {
                        timeit_native_run_in_globals(&cobj, &globals_dict)?;
                    }
                }
            }
            let t1 = py_call(timer.clone(), vec![])?;
            // delta = t1 - t0 (both floats or ints)
            py_sub(&t1, &t0)?
                .as_f64()
                .ok_or_else(|| PyError::type_error("timer returned non-number"))
        } else {
            let t0 = Instant::now();
            match &stmt_prep {
                Prepared::Callable(f) => {
                    for _ in 0..number {
                        py_call(f.clone(), vec![])?;
                    }
                }
                Prepared::Src(code) => {
                    let cobj = PyObjectRef::imm(PyObject::Code(code.clone()));
                    for _ in 0..number {
                        timeit_native_run_in_globals(&cobj, &globals_dict)?;
                    }
                }
            }
            Ok(t0.elapsed().as_secs_f64())
        }
    }

    t_method!("timeit", |args| {
        let self_obj = args.first().cloned().unwrap();
        let (n, kw) = split_kwargs(args);
        if std::env::var("RPY_DBG_TT").is_ok() {
            eprintln!("TT timeit nargs={} kw={:?}", n, kw);
        }
        let number = kw_lookup(&kw, "number")
            .and_then(|v| v.as_i64())
            .or_else(|| args.get(n - n + 1).and_then(|v| v.as_i64()))
            .unwrap_or(1_000_000)
            .max(0) as u64;
        let secs = run_timed(&self_obj, number)?;
        Ok(py_float(secs))
    });

    t_method!("repeat", |args| {
        let self_obj = args.first().cloned().unwrap();
        let (n, kw) = split_kwargs(args);
        // positional fallback: bound-method args are [self, repeat, number]
        let repeat = kw_lookup(&kw, "repeat")
            .and_then(|v| v.as_i64())
            .or_else(|| args.get(1).and_then(|v| v.as_i64()))
            .unwrap_or(5)
            .max(0) as u64;
        let number = kw_lookup(&kw, "number")
            .and_then(|v| v.as_i64())
            .or_else(|| args.get(2).and_then(|v| v.as_i64()))
            .unwrap_or(1_000_000)
            .max(0) as u64;
        let mut times = Vec::new();
        for _ in 0..repeat {
            let secs = run_timed(&self_obj, number)?;
            times.push(py_float(secs));
        }
        Ok(py_list(times))
    });

    // autorange(callback=None) -> (num_loops, time_per_loop).
    // Uses CPython's 1-2-5-per-decade search sequence.
    t_method!("autorange", |args| {
        let self_obj = args.first().cloned().unwrap();
        let callback: Option<PyObjectRef> = args.get(1).and_then(|c| {
            if matches!(&*c.borrow(), PyObject::None) { None } else { Some(c.clone()) }
        }).or_else(|| {
            // kwargs form: callback=<callable> in trailing Dict
            args.last().and_then(|d| {
                let b = d.borrow();
                if let PyObject::Dict(dd) = &*b {
                    dd.items().into_iter()
                        .find(|(k, _)| k.str() == "callback")
                        .map(|(_, v)| v.clone())
                } else { None }
            })
        });
        let report = |callback: &Option<PyObjectRef>, n: usize, secs: f64| -> PyResult<()> {
            if let Some(cb) = callback {
                crate::object::call_function_disposable(
                    cb,
                    vec![py_int(n as i64), py_float(secs)],
                    vec![],
                )?;
            }
            Ok(())
        };
        let mut base = 1usize;
        loop {
            for j in [1usize, 2, 5] {
                let number = base * j;
                let secs = run_timed(&self_obj, number as u64)?;
                report(&callback, number, secs)?;
                if secs >= 0.2 {
                    // CPython returns TOTAL time for the whole run.
                    return Ok(py_tuple(vec![
                        py_int(number as i64),
                        py_float(secs),
                    ]));
                }
            }
            base *= 10;
            if base > 1_000_000_000 {
                return Ok(py_tuple(vec![py_int(base as i64), py_float(0.0)]));
            }
        }
    });

    PyObjectRef::new(PyObject::Type {
        name: "Timer".to_string(),
        dict: Box::new(crate::object::str_map_to_typedict(type_dict)),
        bases: vec![],
        mro: vec![],
    })
}

pub fn create_timeit_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! timeit_func {
        ($name:expr, $func:expr) => {
            d.insert(
                $name.to_string(),
                PyObjectRef::new(PyObject::BuiltinFunction {
                    name: $name.to_string(),
                    func: $func,
                }),
            );
        };
    }

    timeit_func!("timeit", |args| {
        // Trailing Dict = kwargs appended by the dispatcher.
        let (pos, kw) = match args.last() {
            Some(d) => {
                let b = d.borrow();
                if let PyObject::Dict(dd) = &*b {
                    let mut p: Vec<PyObjectRef> = args[..args.len()-1].to_vec();
                    // drop a positional None/placeholder setup if kw supplies one
                    let wrapped = PyObjectRef::imm(PyObject::Dict(dd.clone()));
                    let (_, kwd) = split_kwargs(&[py_none(), wrapped]);
                    if let Some(sv) = kw_lookup(&kwd, "setup") { if p.len() > 1 { p.truncate(1); } }
                    (p, kwd)
                } else { (args.to_vec(), Vec::new()) }
            }
            None => (args.to_vec(), Vec::new()),
        };
        let stmt_v = pos.first().cloned().unwrap_or_else(|| py_str("pass"));
        let setup_v = kw_lookup(&kw, "setup").cloned()
            .or_else(|| pos.get(1).cloned())
            .unwrap_or_else(|| py_str("pass"));
        let timer_v = kw_lookup(&kw, "timer").cloned()
            .or_else(|| pos.get(2).cloned())
            .unwrap_or_else(|| py_none());
        let globals_v = kw_lookup(&kw, "globals").cloned()
            .or_else(|| pos.get(3).cloned())
            .unwrap_or_else(|| py_none());
        let mut cargs = vec![stmt_v, setup_v, timer_v, globals_v];
        let timer_obj = make_timeit_type();
        let inst = crate::object::call_function(&timer_obj, cargs)?;
        let m = inst.borrow().get_attribute("timeit")?;
        let nv_owned = kw_lookup(&kw, "number").map(|v| v.clone())
            .or_else(|| pos.get(1).cloned());
        let mut margs: Vec<PyObjectRef> = vec![];
        if let Some(nv) = nv_owned { margs.push(nv); }
        crate::object::call_function(&m, margs)
    });

    // Also provide a repeat function for convenience — delegates to Timer
    // so callables/timer/globals behave exactly like the class methods.
    timeit_func!("repeat", |args| {
        let (pos, kw) = match args.last() {
            Some(d) => {
                let b = d.borrow();
                if let PyObject::Dict(dd) = &*b {
                    let wrapped = PyObjectRef::imm(PyObject::Dict(dd.clone()));
                    let (_, kwd) = split_kwargs(&[py_none(), wrapped]);
                    let p: Vec<PyObjectRef> = args[..args.len()-1].to_vec();
                    (p, kwd)
                } else { (args.to_vec(), Vec::new()) }
            }
            None => (args.to_vec(), Vec::new()),
        };
        let stmt_v = pos.first().cloned().unwrap_or_else(|| py_str("pass"));
        let setup_v = kw_lookup(&kw, "setup").cloned()
            .or_else(|| pos.get(1).cloned())
            .unwrap_or_else(|| py_str("pass"));
        let timer_v = kw_lookup(&kw, "timer").cloned()
            .or_else(|| pos.get(2).cloned())
            .unwrap_or_else(|| py_none());
        let globals_v = kw_lookup(&kw, "globals").cloned()
            .or_else(|| pos.get(3).cloned())
            .unwrap_or_else(|| py_none());
        let mut cargs = vec![stmt_v, setup_v, timer_v, globals_v];
        let timer_obj = make_timeit_type();
        let inst = crate::object::call_function(&timer_obj, cargs)?;
        let m = inst.borrow().get_attribute("repeat")?;
        let rv_owned = kw_lookup(&kw, "repeat").map(|v| v.clone())
            .or_else(|| pos.get(1).cloned());
        let nv_owned = kw_lookup(&kw, "number").map(|v| v.clone())
            .or_else(|| pos.get(2).cloned());
        let mut margs: Vec<PyObjectRef> = vec![];
        if let Some(rv) = rv_owned { margs.push(rv); }
        if let Some(nv) = nv_owned { margs.push(nv); }
        crate::object::call_function(&m, margs)
    });

    d.insert("Timer".to_string(), make_timeit_type());
    d.insert(
        "reindent".to_string(),
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "reindent".to_string(),
            func: |args| {
                if args.len() < 2 {
                    return Err(PyError::type_error("reindent takes 2 arguments"));
                }
                let src = args[0].str();
                let n = args[1].as_i64().unwrap_or(0).max(0) as usize;
                if n == 0 {
                    // strip common leading whitespace per line, preserving empties
                    let out: Vec<String> = src
                        .lines()
                        .map(|l| l.trim_start().to_string())
                        .collect();
                    return Ok(py_str(&out.join("\n")));
                }
                let pad = " ".repeat(n);
                let out: Vec<String> = src.lines().map(|l| if l.is_empty() { String::new() } else { format!("{}{}", pad, l) }).collect();
                Ok(py_str(&out.join("\n")))
            },
        }),
    );
    d.insert(
        "_compile".to_string(),
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "_compile".to_string(),
            func: |args| {
                let src = args
                    .first()
                    .map(|v| v.str())
                    .ok_or_else(|| PyError::type_error("_compile missing src"))?;
                timeit_native_compile(&src)
            },
        }),
    );
    d.insert(
        "_run_in_globals".to_string(),
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "_run_in_globals".to_string(),
            func: |args| {
                if args.len() < 2 {
                    return Err(PyError::type_error("_run_in_globals needs code, globals"));
                }
                timeit_native_run_in_globals(&args[0], &args[1])
            },
        }),
    );
    d.insert_str("default_number", py_int(1_000_000));
    d.insert_str("default_repeat", py_int(3));

    d
}
