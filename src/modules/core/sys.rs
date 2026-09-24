use crate::object::*;
use num_traits::{Signed, ToPrimitive};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::AtomicI64;

use super::os::dup_std_fd;

// `sys.exc_info()` — a real top-level fn (not an inline closure in
// `create_sys_dict`) so `vm.rs`'s `call_function` can special-case it by
// `fn_addr_eq` pointer identity, running it through the real, live `&mut
// VirtualMachine` instead of `with_vm_mut`. `with_vm_mut`'s `VM_PTR` is set
// unconditionally before any bytecode executes, so calling it from HERE
// (itself only ever reached via a live bytecode CALL) always created a
// second, aliased `&mut VirtualMachine` on top of the one already active —
// real, unconditional UB, confirmed via the simplest possible repro
// (`try: raise ValueError("x") except Exception: sys.exc_info()`)
// reliably segfaulting. The exact same class of bug as the `exec()`/
// `eval()` fix earlier this session — `with_vm_mut` is fundamentally
// unsafe to call from any code path already running under a live VM
// (i.e. virtually always), not just some rare reentrant case.
pub fn sys_exc_info_builtin(_args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let result = crate::object::with_vm_mut(|vm| {
        let tb = if let Some(tb) = vm.exc_traceback.clone() {
            if !matches!(&*tb.borrow(), PyObject::None) {
                Some(tb)
            } else {
                None
            }
        } else {
            None
        };
        // `exc_traceback` is only ever `None` (set at raise time) — the real
        // chain lives on the exception object's own `__traceback__`.
        let tb = tb.or_else(|| {
            vm.exc_value.as_ref().and_then(|v| {
                let r = v.borrow().get_attribute("__traceback__").ok();
                if std::env::var("RPY_DEBUG_EXCINFO").is_ok() {
                    eprintln!(
                        "exc_info tb lookup: {:?}",
                        r.as_ref().map(|t| t.borrow().repr())
                    );
                }
                r.filter(|t| !matches!(&*t.borrow(), PyObject::None))
            })
        });
        py_tuple(vec![
            vm.exc_type.clone().unwrap_or(py_none()),
            vm.exc_value.clone().unwrap_or(py_none()),
            tb.unwrap_or(py_none()),
        ])
    });
    Ok(result.unwrap_or_else(|_| py_tuple(vec![py_none(), py_none(), py_none()])))
}

// `sys.exception()` (3.11+) — same function-pointer-identity special-case
// pattern as `sys_exc_info_builtin` just above (see `call_function`'s own
// matching intercept in `vm.rs`, which is what actually makes this return
// the right value — `with_vm_mut` alone gives the wrong, stale-empty
// result from this reentrant calling context, confirmed via direct repro:
// `except ValueError: sys.exception()` returned `None` instead of the real
// exception instance, using ONLY this `with_vm_mut`-based body without the
// intercept). Kept as a real, named (not closure) `fn` so `call_function`
// can identify it by pointer, same as `sys_exc_info_builtin`.
pub fn sys_exception_builtin(_args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let result = crate::object::with_vm_mut(|vm| vm.exc_value.clone().unwrap_or(py_none()));
    Ok(result.unwrap_or_else(|_| py_none()))
}

thread_local! {
    // `sys.settrace`/`gettrace`: stores the current global trace function.
    // This does NOT actually fire 'call'/'line'/'return'/'exception' trace
    // events during execution (that needs deep VM instrumentation — a much
    // larger feature, not attempted here) — it only makes the get/set
    // protocol itself work, which is enough for the extremely common
    // `self.addCleanup(sys.settrace, sys.gettrace())` /
    // `sys.settrace(None); assert sys.gettrace() is None` setup/teardown
    // pattern (real trigger: `test_sys_settrace.py`'s own test fixtures)
    // to stop raising `AttributeError: 'module' object has no attribute
    // 'settrace'` on every single test in the file, rather than crashing
    // before even reaching whatever the test itself checks.
    static CURRENT_TRACE_FUNC: std::cell::RefCell<Option<PyObjectRef>> = std::cell::RefCell::new(None);

    // `sys.setprofile`/`getprofile`: same get/set-only stub shape as
    // `CURRENT_TRACE_FUNC` above (no real 'call'/'return'/'c_call'/
    // 'c_return' event dispatch — that needs the same deep VM
    // instrumentation `settrace` already documents as out of scope). Exists
    // so `Lib/profile.py`'s pure-Python `Profile.runctx`/`runcall`
    // (`sys.setprofile(self.dispatcher)` / `sys.setprofile(None)`) don't
    // raise `AttributeError: module 'sys' has no attribute 'setprofile'`
    // before even reaching the statement being profiled.
    static CURRENT_PROFILE_FUNC: std::cell::RefCell<Option<PyObjectRef>> = std::cell::RefCell::new(None);
}

pub fn sys_setprofile_builtin(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let func = args.first().cloned().unwrap_or_else(py_none);
    CURRENT_PROFILE_FUNC.with(|f| {
        *f.borrow_mut() = if matches!(&*func.borrow(), PyObject::None) {
            None
        } else {
            Some(func)
        };
    });
    Ok(py_none())
}

pub fn sys_getprofile_builtin(_args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    Ok(CURRENT_PROFILE_FUNC
        .with(|f| f.borrow().clone())
        .unwrap_or_else(py_none))
}

thread_local! {
    // `sys.intern` table: value -> canonical instance. CPython interns
    // permanently; strong refs here match that (test_marshal checks
    // id() identity across marshal round-trips).
    static INTERN_TABLE: std::cell::RefCell<HashMap<String, PyObjectRef>> =
        std::cell::RefCell::new(HashMap::new());
}

pub fn sys_intern_builtin(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let s = args
        .first()
        .ok_or_else(|| PyError::type_error("intern() missing string argument"))?;
    let val = s.str();
    // Only strings are internable; non-strings raise TypeError like CPython.
    {
        let b = s.borrow();
        match &*b {
            PyObject::Str(_) => {}
            _ => return Err(PyError::type_error("sys.intern() argument must be str")),
        }
    }
    INTERN_TABLE.with(|t| {
        if let Some(v) = t.borrow().get(&val).cloned() {
            return Ok(v);
        }
        t.borrow_mut().insert(val, s.clone());
        Ok(s.clone())
    })
}

pub fn sys_settrace_builtin(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let func = args.first().cloned().unwrap_or_else(py_none);
    CURRENT_TRACE_FUNC.with(|f| {
        *f.borrow_mut() = if matches!(&*func.borrow(), PyObject::None) {
            None
        } else {
            Some(func)
        };
    });
    Ok(py_none())
}

pub fn sys_gettrace_builtin(_args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    Ok(CURRENT_TRACE_FUNC
        .with(|f| f.borrow().clone())
        .unwrap_or_else(py_none))
}

// `sys._getframe(depth=0)` — `vm.rs`'s `call_function` special-cases this
// (matched by `fn_addr_eq` against this exact function) to run against the
// REAL, live `&mut VirtualMachine` instead of ever reaching this body — see
// that call site's own doc comment for the full story (this was previously
// a no-op always returning `None`, breaking `Lib/test/support/warnings_
// helper.py`'s `_filterwarnings`/`check_warnings`, used pervasively by
// warning-related tests across the corpus). This `with_vm_mut`-based body
// only serves as the identity target for `fn_addr_eq` plus a safety-net
// fallback for any call shape that somehow bypasses that special-casing.
pub fn sys_getframe_builtin(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let depth = args.first().and_then(|a| a.as_i64()).unwrap_or(0);
    crate::object::with_vm_mut(|vm| -> PyResult<PyObjectRef> {
        if depth < 0 {
            return Err(PyError::value_error("call stack is not deep enough"));
        }
        let raw_idx = (vm.frames.len() as i64) - 1 - depth;
        let idx = if raw_idx >= 0 {
            raw_idx as usize
        } else {
            // Clamp to the deepest available frame (generator frames run in
            // a disposable VM with only their own frame — see the vm.rs
            // special-case's own comment).
            0
        };
        if vm.frames.get(idx).is_none() {
            return Err(PyError::value_error("call stack is not deep enough"));
        }
        // Reuse the frame's cached Python `frame` object when it exists so
        // `sys._getframe()` returns the SAME object an exception traceback
        // captured for that live frame (`tb.tb_frame is sys._getframe()`).
        vm.frame_object(idx)
            .ok_or_else(|| PyError::value_error("call stack is not deep enough"))
    })?
}

pub fn sys_getrecursionlimit_builtin(_args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let result = crate::object::with_vm_mut(|vm| py_int(vm.recursion_limit as i64));
    Ok(result.unwrap_or_else(|_| py_int(1000)))
}

pub fn sys_setrecursionlimit_builtin(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let n = args
        .get(0)
        .and_then(|a| a.as_i64())
        .ok_or_else(|| PyError::type_error("setrecursionlimit() requires an integer argument"))?;
    if n < 1 {
        return Err(PyError::value_error(
            "recursion limit must be greater or equal than 1",
        ));
    }
    let _ = crate::object::with_vm_mut(|vm| {
        vm.recursion_limit = n as usize;
    });
    Ok(py_none())
}

// `sys.excepthook`/`sys.__excepthook__` — the default uncaught-exception
// reporter: `excepthook(exc_type, exc_value, exc_tb)` walks the (now real)
// traceback chain and prints a CPython-style report. The report BUILDING is
// VM-independent (plain object attribute reads); the actual sys.stderr write
// happens in `call_function`'s pointer-identified intercept in vm.rs (where
// the live `&mut VirtualMachine` is available — `with_vm_mut` is UB from
// inside a live call chain, see that file's own comments). This fallback
// body only fires when the intercept doesn't (e.g. called indirectly).
pub fn sys_excepthook_builtin(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    let out = build_excepthook_report(args)?;
    if !out.is_empty() {
        use std::io::Write;
        let _ = std::io::stderr().write_all(out.as_bytes());
    }
    Ok(py_none())
}

/// `sys.breakpointhook` / `sys.__breakpointhook__` — CPython stores the
/// SAME builtin function object under both names (see
/// `COPY_SYS_ATTR("__breakpointhook__", "breakpointhook")` in
/// `_PySys_InitCore`, `Python/sysmodule.c`), and `breakpoint()` calls
/// whatever `sys.breakpointhook` currently holds. That function
/// (`sys_breakpointhook` in CPython) re-resolves `$PYTHONBREAKPOINT` on
/// EVERY call and then calls the resolved hook with `breakpoint()`'s own
/// arguments — an import plus a real call, both of which need a live
/// `&mut VirtualMachine`. So the real body lives in
/// `breakpointhook_resolve_and_call`, invoked from `call_function`'s
/// pointer-identified intercept in `vm/call.rs`; this plain-function form
/// is only the identity/marker that interception matches on.
pub fn sys_breakpointhook_builtin(_args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    Err(PyError::runtime_error(
        "sys.breakpointhook() requires interpreter access",
    ))
}

/// `sys_breakpointhook` (CPython `Python/sysmodule.c`) with a live VM:
/// resolve `$PYTHONBREAKPOINT` (empty/unset → `pdb.set_trace`, `"0"` →
/// no-op), import the named module, fetch the attribute, and call it with
/// the caller's own args/kwargs. A failed import or a missing attribute is
/// warned about and ignored (`RuntimeWarning: Ignoring unimportable
/// $PYTHONBREAKPOINT: "..."`), matching CPython's `warn:` label; any
/// OTHER error from the import propagates.
pub(crate) fn breakpointhook_resolve_and_call(
    vm: &mut crate::vm::VirtualMachine,
    args: &[PyObjectRef],
    keywords: &[(String, PyObjectRef)],
) -> PyResult<PyObjectRef> {
    let raw = std::env::var("PYTHONBREAKPOINT").unwrap_or_default();
    let envar = if raw.is_empty() {
        "pdb.set_trace".to_string()
    } else {
        raw
    };
    if envar == "0" {
        return Ok(py_none());
    }
    let (modulepath, attrname) = match envar.rfind('.') {
        None => ("builtins".to_string(), envar.clone()),
        // Leading or trailing dot is malformed ("." / "..foo" / "int.") —
        // CPython jumps straight to the warning branch.
        Some(0) => return breakpointhook_warn(&envar),
        Some(i) => (envar[..i].to_string(), envar[i + 1..].to_string()),
    };
    if attrname.is_empty() {
        return breakpointhook_warn(&envar);
    }
    let module = match crate::object::import_impl(vm, &modulepath, modulepath.contains('.'), false)
    {
        Ok(m) => m,
        Err(e) if is_import_error(&e) => return breakpointhook_warn(&envar),
        Err(e) => return Err(e),
    };
    let hook = match module.borrow().get_attribute(&attrname) {
        Ok(h) => h,
        // `_PyErr_ExceptionMatches(tstate, PyExc_AttributeError)` — which
        // also covers a real raised `AttributeError` object, the shape a
        // module's own `__getattr__` produces.
        Err(e) if is_attribute_error(&e) => return breakpointhook_warn(&envar),
        Err(e) => return Err(e),
    };
    vm.call_function(hook, args.to_vec(), keywords.to_vec())
}

/// The `AttributeError` test counterpart of `is_import_error` — a
/// module-level `__getattr__` raising a real `AttributeError` arrives as
/// `PyError::Exception`, not the enum variant.
fn is_attribute_error(e: &PyError) -> bool {
    match e {
        PyError::AttributeError(_) => true,
        PyError::Exception(name, _) => name == "AttributeError",
        _ => false,
    }
}

/// CPython tests with `_PyErr_ExceptionMatches(tstate, PyExc_ImportError)`,
/// which also matches `ModuleNotFoundError` — this interpreter's
/// `ImportError` is an enum variant AND (for module-not-found) an
/// `Exception` carrying the subclass name.
fn is_import_error(e: &PyError) -> bool {
    match e {
        PyError::ImportError(_) => true,
        PyError::Exception(name, _) => name == "ImportError" || name == "ModuleNotFoundError",
        _ => false,
    }
}

fn breakpointhook_warn(envar: &str) -> PyResult<PyObjectRef> {
    crate::modules::dev::warnings_emit(
        &format!("Ignoring unimportable $PYTHONBREAKPOINT: \"{envar}\""),
        "RuntimeWarning",
    );
    Ok(py_none())
}

/// Build the CPython-style traceback report string for `(exc_type,
/// exc_value, exc_tb)` (no VM access needed).
pub(crate) fn build_excepthook_report(args: &[PyObjectRef]) -> PyResult<String> {
    let (exc_type, exc_value, exc_tb) = match args.len() {
        3 => (&args[0], &args[1], &args[2]),
        0 => {
            return Err(PyError::type_error(
                "excepthook() takes exactly 3 arguments (0 given)",
            ))
        }
        _ => {
            return Err(PyError::type_error(format!(
                "excepthook() takes exactly 3 arguments ({} given)",
                args.len()
            )))
        }
    };
    let mut out = String::from("Traceback (most recent call last):\n");
    let mut last_source_line_end = out.len(); // track where to insert caret
    let mut tb = if matches!(&*exc_tb.borrow(), PyObject::None) {
        None
    } else {
        Some(exc_tb.clone())
    };
    let mut count = 0;
    while let Some(node) = tb {
        if count >= 100 {
            out.push_str("  ...\n");
            break;
        }
        let (filename, lineno, name) = {
            let b = node.borrow();
            let frame = b.get_attribute("tb_frame").ok();
            let lineno = b.get_attribute("tb_lineno").ok().and_then(|l| l.as_i64());
            let (filename, name) = match frame {
                Some(f) => {
                    let fb = f.borrow();
                    let code = fb.get_attribute("f_code").ok();
                    match code {
                        Some(c) => {
                            let cb = c.borrow();
                            (
                                cb.get_attribute("co_filename")
                                    .ok()
                                    .map(|s| s.str())
                                    .unwrap_or_else(|| "<unknown>".to_string()),
                                cb.get_attribute("co_name")
                                    .ok()
                                    .map(|s| s.str())
                                    .unwrap_or_else(|| "?".to_string()),
                            )
                        }
                        None => ("<unknown>".to_string(), "?".to_string()),
                    }
                }
                None => ("<unknown>".to_string(), "?".to_string()),
            };
            (filename, lineno.unwrap_or(0), name)
        };
        out.push_str(&format!(
            "  File \"{}\", line {}, in {}\n",
            filename, lineno, name
        ));
        // Source line, when the file is readable (CPython prints the
        // offending line with an indent).
        if lineno > 0 && !filename.is_empty() {
            if let Ok(src) = std::fs::read_to_string(&filename) {
                if let Some(line) = src.lines().nth(lineno as usize - 1) {
                    out.push_str(&format!("    {}\n", line));
                    last_source_line_end = out.len();
                }
            }
        }
        count += 1;
        tb = node.borrow().get_attribute("tb_next").ok().and_then(|n| {
            if matches!(&*n.borrow(), PyObject::None) {
                None
            } else {
                Some(n)
            }
        });
    }
    // `TypeName: message` — CPython prints `<exception str() failed>` when
    // str(exc) itself raises; our str() swallows such errors, so the message
    // is used directly.
    let typ_name = exc_type
        .borrow()
        .get_attribute("__name__")
        .map(|n| n.str())
        .unwrap_or_else(|_| exc_type.str());
    // Fallible str(): a custom __str__ that raises yields CPython's
    // `<exception str() failed>` marker (test_unhandled's BrokenStrException),
    // instead of silently falling back to the repr.
    let message = {
        let custom = match &*exc_value.borrow() {
            PyObject::Instance { typ, .. } => crate::object::lookup_dunder_via_mro(typ, "__str__")
                .or_else(|| crate::object::lookup_dunder_via_mro(typ, "__repr__")),
            _ => None,
        };
        if let Some(f) = custom {
            match crate::object::call_bound_method(f, exc_value.clone(), vec![]) {
                Ok(r) => r.str(),
                Err(_) => "<exception str() failed>".to_string(),
            }
        } else {
            exc_value.str()
        }
    };
    // SyntaxError caret display: insert the '^' indicator line after the
    // source line but before the error message.
    if typ_name == "SyntaxError" || typ_name == "IndentationError" || typ_name == "TabError" {
        let eb = exc_value.borrow();
        let offset = eb.get_attribute("offset").ok().and_then(|v| v.as_i64());
        let text = eb.get_attribute("text").ok().map(|v| v.str());
        let end_offset = eb.get_attribute("end_offset").ok().and_then(|v| v.as_i64());
        drop(eb);

        if let (Some(offset), Some(ref text)) = (offset, text) {
            // Mirrors CPython's `TracebackException._format_syntax_error`'s
            // offset-clamping formula (see `Lib/traceback.py` upstream):
            // clamp `offset`/`end_offset` against the source text's length
            // and against each other BEFORE turning them into 0-based
            // column indices. The previous version cast `offset - 1` /
            // `end - 1` straight to `usize` — for a deliberately-negative
            // or nonsensical end_offset (real CPython test:
            // `test_exceptions.py`'s `test_range_of_offsets`, e.g.
            // `end_offset=-2` or `end_offset` before `offset`) that
            // subtraction underflows to a huge `usize`, and
            // `"^".repeat(huge)` aborts the process with a `Vec` capacity
            // overflow instead of raising/rendering anything. Doing the
            // whole computation in `i64` and only converting to `usize`
            // after clamping to `>= 0` makes that impossible.
            let rtext = text.trim_end_matches('\n');
            let ltext = rtext.trim_start_matches(['\n', '\x0c', ' ']);
            let spaces = (rtext.len() - ltext.len()) as i64;
            let mut off = offset;
            let mut end_off = match end_offset {
                Some(e) if e != 0 => e,
                _ => offset,
            };
            if !rtext.is_empty() {
                let rlen = rtext.len() as i64;
                if off > rlen {
                    off = rlen + 1;
                }
                if end_off > rlen {
                    end_off = rlen + 1;
                }
            }
            if off >= end_off || end_off < 0 {
                end_off = off + 1;
            }
            let colno = off - 1 - spaces;
            let end_colno = end_off - 1 - spaces;
            // CPython only shows carets at all when the (clamped) start
            // column is non-negative — a negative/zero raw offset (or one
            // that clamps to before the start of the stripped text) means
            // no caret line, just the source line.
            if colno >= 0 {
                let indent = "    "; // 4-space indent
                let caret_len = std::cmp::max(0, end_colno - colno) as usize;
                let caret_line = format!(
                    "{}{}{}\n",
                    indent,
                    " ".repeat(colno as usize),
                    "^".repeat(caret_len)
                );
                // Insert caret after the source line
                out.insert_str(last_source_line_end, &caret_line);
            }
        }
    }

    if message.is_empty() {
        out.push_str(&format!("{}\n", typ_name));
    } else {
        out.push_str(&format!("{}: {}\n", typ_name, message));
    }
    Ok(out)
}

pub fn create_sys_dict(argv: Vec<String>) -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! sys_func {
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
    sys_func!("exit", |args| {
        if args.is_empty() {
            return Err(PyError::SystemExit(0));
        }
        if let Some(i) = args[0].as_i64() {
            return Err(PyError::SystemExit(i as i32));
        }
        if matches!(&*args[0].borrow(), PyObject::None) {
            return Err(PyError::SystemExit(0));
        }
        Err(PyError::Exception(
            "SystemExit".to_string(),
            PyObjectRef::new(PyObject::Exception {
                typ: "SystemExit".to_string(),
                args: vec![args[0].clone()],
                cause: None,
                suppress_context: false,
                context: None,
                traceback: None,
                extra: None,
            }),
        ))
    });
    // sys.audit — CPython audit hook stub. No-op in this interpreter.
    sys_func!("audit", |_args| Ok(py_none()));
    sys_func!("addaudithook", |_args| Ok(py_none()));
    d.insert(
        "intern".to_string(),
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "intern".to_string(),
            func: sys_intern_builtin,
        }),
    );
    sys_func!("displayhook", |args| {
        if args.is_empty() {
            return Ok(py_none());
        }
        let val = &args[0];
        if matches!(&*val.borrow(), PyObject::None) {
            return Ok(py_none());
        }
        println!("{}", val.repr());
        Ok(py_none())
    });
    // `sys.excepthook`/`sys.__excepthook__` — the default uncaught-exception
    // reporter. Called as `excepthook(exc_type, exc_value, exc_tb)`; walks
    // the (now real) traceback chain and prints a CPython-style report to
    // stderr, including the source line when the file is readable. Real
    // trigger: CPython's own test_exceptions (test_unhandled,
    // test_issue45826, ...) calling `sys.__excepthook__(*sys.exc_info())`
    // and asserting on the report.
    sys_func!("excepthook", sys_excepthook_builtin);
    sys_func!("__excepthook__", sys_excepthook_builtin);
    // `sys.breakpointhook`/`sys.__breakpointhook__` — the SAME function
    // object in both names (CPython copies it during `_PySys_InitCore`);
    // `breakpoint()` calls `sys.breakpointhook(*args, **kwargs)`, which
    // resolves `$PYTHONBREAKPOINT` per call (real body needs the live VM —
    // see `breakpointhook_resolve_and_call` and its intercept in
    // `vm/call.rs`). Without these attributes, `breakpoint()` itself,
    // `sys.__breakpointhook__` reads, and test_builtin's whole
    // `TestBreakpoint` class (12 tests) fail with AttributeError.
    sys_func!("breakpointhook", sys_breakpointhook_builtin);
    sys_func!("__breakpointhook__", sys_breakpointhook_builtin);
    // `sys.unraisablehook` (3.8+) — called by the interpreter when an
    // weakref callback, ...) and would otherwise just be silently dropped.
    // This interpreter has no internal machinery that actually detects and
    // invokes this hook, but real code (and test infra) routinely reads/
    // reassigns `sys.unraisablehook` itself (`sys.unraisablehook = my_hook`
    // to capture what WOULD be reported) — missing the attribute entirely
    // raised `AttributeError` just from that, before any such machinery
    // would even matter. Default mirrors real CPython's own default
    // behavior: print a short summary to stderr.
    sys_func!("unraisablehook", |args| {
        if let Some(unraisable) = args.first() {
            eprintln!("Exception ignored: {}", unraisable.repr());
        }
        Ok(py_none())
    });
    d.insert_str(
        "argv",
        py_list(argv.into_iter().map(|s| py_str(&s)).collect()),
    );
    d.insert_str("path", py_list(vec![]));
    d.insert_str("modules", py_dict());
    d.insert_str("warnoptions", py_list(vec![]));
    // Real CPython's `sys.version` has a specific, stable, parseable shape
    // (`<version> (<branch>, <build date>) [<compiler>]`) that real stdlib
    // code parses via regex — `Lib/platform.py`'s `_sys_version()`, used by
    // `platform.python_version()`/`platform.platform()`, real trigger:
    // `test_wsgiref.py` (which calls `platform.python_version()` at import
    // time) crashed entirely on the old, non-matching
    // `"3.12.0 (RustPython 0.1.0)"` shape. Keeps "RustPython" visible (in
    // the compiler bracket) so every `'rustpython' in sys.version.lower()`
    // test-skip marker across the corpus still matches, and keeps the
    // version number consistent with `sys.version_info` below (unchanged).
    d.insert_str(
        "version",
        py_str("3.12.0 (main, Sep 12 2026, 00:00:00) [RustPython 0.1.0]"),
    );
    // `sys._git` — a real CPython attribute (CPython/Python/sysmodule.c),
    // a 3-tuple (project, tag, version) describing the VCS build info.
    // `Lib/platform.py`'s module-level code and `test_platform.py`'s
    // `PlatformTest.setUp` do `sys._git` unconditionally (not behind
    // hasattr), so its absence raised AttributeError before any test in
    // that class could even run.
    d.insert_str(
        "_git",
        py_tuple(vec![py_str("CPython"), py_str(""), py_str("")]),
    );
    // sys.version_info — structseq (tuple + attributes) like
    // time.struct_time: supports both v[0] and v.major. Plain tuple
    // broke attribute access (`'tuple' object has no attribute
    // 'major'`, test_embed and many version gates).
    {
        let fields: &[(&str, PyObjectRef)] = &[
            ("major", py_int(3)),
            ("minor", py_int(12)),
            ("micro", py_int(0)),
            ("releaselevel", py_str("final")),
            ("serial", py_int(0)),
        ];
        let mut type_dict: HashMap<String, PyObjectRef> = HashMap::new();
        type_dict.insert_str(
            "__getitem__",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "__getitem__".to_string(),
                func: |args| {
                    if args.len() < 2 {
                        return Err(PyError::type_error(
                            "__getitem__() takes exactly one argument",
                        ));
                    }
                    let order = ["major", "minor", "micro", "releaselevel", "serial"];
                    if let PyObject::Instance { dict, .. } = &*args[0].borrow() {
                        if let PyObject::Slice { start, stop, step } = &*args[1].borrow() {
                            use crate::object::extract_slice_fields;
                            let n = order.len() as isize;
                            let (s0, s1, step_v) = extract_slice_fields(start, stop, step)?;
                            let norm = |v: Option<isize>, dflt: isize| -> isize {
                                match v {
                                    Some(v) => {
                                        if v < 0 {
                                            (n + v).max(0)
                                        } else {
                                            v.min(n)
                                        }
                                    }
                                    None => dflt,
                                }
                            };
                            let mut out = Vec::new();
                            if step_v > 0 {
                                let (mut i, stop) = (norm(s0, 0), norm(s1, n));
                                while i < stop {
                                    if let Some(name) = order.get(i as usize) {
                                        out.push(
                                            dict.get_str(name).cloned().unwrap_or_else(py_none),
                                        );
                                    }
                                    i += step_v;
                                }
                            } else {
                                let (mut i, stop) = (
                                    match s0 {
                                        Some(v) => {
                                            if v < 0 {
                                                (n + v).max(-1)
                                            } else {
                                                v.min(n - 1)
                                            }
                                        }
                                        None => n - 1,
                                    },
                                    norm(s1, -1),
                                );
                                while i > stop {
                                    if let Some(name) = order.get(i as usize) {
                                        out.push(
                                            dict.get_str(name).cloned().unwrap_or_else(py_none),
                                        );
                                    }
                                    i += step_v;
                                }
                            }
                            return Ok(py_tuple(out));
                        }
                        let idx = args[1]
                            .as_i64()
                            .ok_or_else(|| PyError::type_error("tuple indices must be integers"))?;
                        let i = if idx < 0 {
                            idx + order.len() as i64
                        } else {
                            idx
                        };
                        let name = order.get(i as usize).ok_or_else(|| {
                            PyError::index_error("version_info index out of range")
                        })?;
                        Ok(dict.get_str(name).cloned().unwrap_or_else(py_none))
                    } else {
                        Err(PyError::runtime_error("__getitem__ on non-version_info"))
                    }
                },
            }),
        );
        type_dict.insert_str(
            "__len__",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "__len__".to_string(),
                func: |_args| Ok(py_int(5)),
            }),
        );
        type_dict.insert_str(
            "__iter__",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "__iter__".to_string(),
                func: |args| {
                    if let PyObject::Instance { dict, .. } = &*args[0].borrow() {
                        let order = ["major", "minor", "micro", "releaselevel", "serial"];
                        let items: Vec<PyObjectRef> = order
                            .iter()
                            .map(|f| dict.get_str(f).cloned().unwrap_or_else(py_none))
                            .collect();
                        Ok(PyObjectRef::new(PyObject::ListIter {
                            list: items,
                            index: 0,
                        }))
                    } else {
                        Err(PyError::runtime_error("__iter__ on non-version_info"))
                    }
                },
            }),
        );
        type_dict.insert_str(
            "__repr__",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "__repr__".to_string(),
                func: |args| {
                    if let PyObject::Instance { dict, .. } = &*args[0].borrow() {
                        let major = dict.get_str("major").and_then(|v| v.as_i64()).unwrap_or(3);
                        let minor = dict.get_str("minor").and_then(|v| v.as_i64()).unwrap_or(12);
                        let micro = dict.get_str("micro").and_then(|v| v.as_i64()).unwrap_or(0);
                        let rel = dict
                            .get_str("releaselevel")
                            .map(|v| v.str())
                            .unwrap_or_else(|| "final".to_string());
                        let serial = dict.get_str("serial").and_then(|v| v.as_i64()).unwrap_or(0);
                        Ok(py_str(&format!(
                            "sys.version_info(major={}, minor={}, micro={}, releaselevel='{}', serial={})",
                            major, minor, micro, rel, serial
                        )))
                    } else {
                        Ok(py_str("sys.version_info(...)"))
                    }
                },
            }),
        );
        // Rich comparison delegates to tuple semantics (version gates like
        // `sys.version_info >= (3, 8)` are ubiquitous; without these the
        // structseq switch regressed them to TypeError).
        fn vi_as_tuple(obj: &PyObjectRef) -> Option<Vec<PyObjectRef>> {
            if let PyObject::Instance { dict, .. } = &*obj.borrow() {
                let order = ["major", "minor", "micro", "releaselevel", "serial"];
                if dict.get_str("major").is_none() {
                    return None;
                }
                Some(
                    order
                        .iter()
                        .map(|f| dict.get_str(f).cloned().unwrap_or_else(py_none))
                        .collect(),
                )
            } else {
                None
            }
        }
        fn vi_compare(args: &[PyObjectRef], op: u32) -> PyResult<PyObjectRef> {
            if args.len() < 2 {
                return Err(PyError::type_error("comparison missing argument"));
            }
            let self_tup = vi_as_tuple(&args[0])
                .ok_or_else(|| PyError::runtime_error("comparison on non-version_info"))?;
            let self_tup = py_tuple(self_tup);
            let other = &args[1];
            let other_tup: Option<PyObjectRef> = match &*other.borrow() {
                PyObject::Tuple(_) | PyObject::List(_) => Some(other.clone()),
                PyObject::Instance { .. } => vi_as_tuple(other).map(py_tuple),
                _ => None,
            };
            match other_tup {
                Some(o) => crate::object::py_compare(&self_tup, &o, op),
                None => {
                    if op == 2 {
                        Ok(py_bool(false))
                    } else if op == 5 {
                        Ok(py_bool(true))
                    } else {
                        Err(PyError::type_error(format!(
                            "'{}' not supported between instances of 'version_info' and '{}'",
                            match op {
                                0 => "<",
                                1 => "<=",
                                3 => ">=",
                                4 => ">",
                                _ => "!=",
                            },
                            other.get_type_name(),
                        )))
                    }
                }
            }
        }
        // Six explicit dunders (BuiltinFunction holds a fn pointer, so
        // capturing closures can't be used — one closure per op).
        type_dict.insert_str(
            "__eq__",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "__eq__".to_string(),
                func: |args| vi_compare(args, 2),
            }),
        );
        type_dict.insert_str(
            "__ne__",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "__ne__".to_string(),
                func: |args| vi_compare(args, 5),
            }),
        );
        type_dict.insert_str(
            "__lt__",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "__lt__".to_string(),
                func: |args| vi_compare(args, 0),
            }),
        );
        type_dict.insert_str(
            "__le__",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "__le__".to_string(),
                func: |args| vi_compare(args, 1),
            }),
        );
        type_dict.insert_str(
            "__gt__",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "__gt__".to_string(),
                func: |args| vi_compare(args, 4),
            }),
        );
        type_dict.insert_str(
            "__ge__",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "__ge__".to_string(),
                func: |args| vi_compare(args, 3),
            }),
        );
        let mut vi_dict = AttrMap::new();
        for (k, v) in fields {
            vi_dict.insert_str(k, v.clone());
        }
        d.insert_str(
            "version_info",
            PyObjectRef::new(PyObject::Instance {
                typ: PyObjectRef::new(PyObject::Type {
                    name: "version_info".to_string(),
                    dict: Box::new(str_map_to_typedict(type_dict)),
                    bases: vec![],
                    mro: vec![],
                }),
                dict: vi_dict,
            }),
        );
    }
    d.insert_str("float_repr_style", py_str("short"));
    d.insert_str("hexversion", py_int(0x030c0000));
    d.insert_str(
        "copyright",
        py_str(
            "Copyright (c) 2001-2025 Python Software Foundation.\nAll Rights Reserved.\n\nCopyright (c) 2000 BeOpen.com.\nAll Rights Reserved.\n\nCopyright (c) 1995-2001 Corporation for National Research Initiatives.\nAll Rights Reserved.\n\nCopyright (c) 1991-1995 Stichting Mathematisch Centrum, Amsterdam.\nAll Rights Reserved.",
        ),
    );
    // sys.flags — real CPython exposes this as a structseq (tuple +
    // attribute access). A plain Instance with these as attributes is
    // enough for real code that reads specific flags by name (e.g.
    // unittest's runner checking `sys.flags.dev_mode` transitively) — all
    // false/zero, matching "no special flags" for a script run normally.
    {
        let mut flags_dict = AttrMap::new();
        for flag in [
            "debug",
            "inspect",
            "interactive",
            "optimize",
            "dont_write_bytecode",
            "no_user_site",
            "no_site",
            "ignore_environment",
            "verbose",
            "bytes_warning",
            "quiet",
            "hash_randomization",
            "isolated",
            "dev_mode",
            "utf8_mode",
            "safe_path",
            "warn_default_encoding",
            "context_aware_warnings",
            "thread_inherit_context",
        ] {
            flags_dict.insert(flag.to_string(), py_int(0));
        }
        flags_dict.insert_str("gil", py_int(1));
        flags_dict.insert_str("hash_randomization", py_int(1));
        flags_dict.insert_str("int_max_str_digits", py_int(4300));
        d.insert_str(
            "flags",
            PyObjectRef::new(PyObject::Instance {
                typ: PyObjectRef::new(PyObject::Type {
                    name: "flags".to_string(),
                    dict: Box::new(str_map_to_typedict(HashMap::new())),
                    bases: vec![],
                    mro: vec![],
                }),
                dict: flags_dict,
            }),
        );
    }
    {
        // sys.hash_info — a real CPython structseq describing the hash
        // algorithm's parameters. Values match this interpreter's actual
        // hashing (a plain 64-bit `usize` computed directly, no SipHash
        // randomization) — `width`/`hash_bits` are the two fields real
        // code actually reads (`test.support`'s own `NHASHBITS = sys.
        // hash_info.width`); the rest are filled in for completeness/
        // structural parity with real CPython rather than because
        // anything here depends on them being exact.
        let mut hash_info_dict = AttrMap::new();
        hash_info_dict.insert_str("width", py_int(64));
        hash_info_dict.insert_str("modulus", py_int((1i64 << 61) - 1));
        hash_info_dict.insert_str("inf", py_int(314159));
        hash_info_dict.insert_str("nan", py_int(0));
        hash_info_dict.insert_str("imag", py_int(1000003));
        hash_info_dict.insert_str("algorithm", py_str("siphash13"));
        hash_info_dict.insert_str("hash_bits", py_int(64));
        hash_info_dict.insert_str("seed_bits", py_int(128));
        hash_info_dict.insert_str("cutoff", py_int(0));
        d.insert_str(
            "hash_info",
            PyObjectRef::new(PyObject::Instance {
                typ: PyObjectRef::new(PyObject::Type {
                    name: "hash_info".to_string(),
                    dict: Box::new(str_map_to_typedict(HashMap::new())),
                    bases: vec![],
                    mro: vec![],
                }),
                dict: hash_info_dict,
            }),
        );
    }
    {
        // sys.int_info — describes the internal representation of Python
        // `int`. This interpreter backs arbitrary-precision ints with
        // `num-bigint`, not CPython's own 30-bit-digit array — these
        // values are CPython's OWN real constants (`bits_per_digit=30`,
        // `sizeof_digit=4`), reported for compatibility with code that
        // merely inspects them (e.g. `sys.int_info.bits_per_digit`)
        // without depending on this interpreter's actual internal
        // representation matching bit-for-bit.
        let mut int_info_dict = AttrMap::new();
        int_info_dict.insert_str("bits_per_digit", py_int(30));
        int_info_dict.insert_str("sizeof_digit", py_int(4));
        int_info_dict.insert_str("default_max_str_digits", py_int(4300));
        int_info_dict.insert_str("str_digits_check_threshold", py_int(640));
        d.insert_str(
            "int_info",
            PyObjectRef::new(PyObject::Instance {
                typ: PyObjectRef::new(PyObject::Type {
                    name: "int_info".to_string(),
                    dict: Box::new(str_map_to_typedict(HashMap::new())),
                    bases: vec![],
                    mro: vec![],
                }),
                dict: int_info_dict,
            }),
        );
    }
    {
        // sys.thread_info — describes the threading implementation. This
        // interpreter's own `threading` module is backed by real OS
        // threads (`std::thread`), which is exactly what CPython's own
        // "pthread" report describes on any POSIX platform.
        let mut thread_info_dict = AttrMap::new();
        thread_info_dict.insert_str("name", py_str("pthread"));
        thread_info_dict.insert_str("lock", py_str("mutex+cond"));
        thread_info_dict.insert_str("version", py_none());
        d.insert_str(
            "thread_info",
            PyObjectRef::new(PyObject::Instance {
                typ: PyObjectRef::new(PyObject::Type {
                    name: "thread_info".to_string(),
                    dict: Box::new(str_map_to_typedict(HashMap::new())),
                    bases: vec![],
                    mro: vec![],
                }),
                dict: thread_info_dict,
            }),
        );
    }
    {
        // sys.float_info — real CPython structseq describing the platform
        // `double` (matches Rust `f64`, IEEE 754 binary64 — same values
        // real CPython reports on any IEEE-754 platform, which is
        // effectively all of them).
        let mut float_info_dict = AttrMap::new();
        float_info_dict.insert_str("max", py_float(f64::MAX));
        float_info_dict.insert_str("max_exp", py_int(1024));
        float_info_dict.insert_str("max_10_exp", py_int(308));
        float_info_dict.insert_str("min", py_float(f64::MIN_POSITIVE));
        float_info_dict.insert_str("min_exp", py_int(-1021));
        float_info_dict.insert_str("min_10_exp", py_int(-307));
        float_info_dict.insert_str("dig", py_int(15));
        float_info_dict.insert_str("mant_dig", py_int(53));
        float_info_dict.insert_str("epsilon", py_float(f64::EPSILON));
        float_info_dict.insert_str("radix", py_int(2));
        float_info_dict.insert_str("rounds", py_int(1));
        d.insert_str(
            "float_info",
            PyObjectRef::new(PyObject::Instance {
                typ: PyObjectRef::new(PyObject::Type {
                    name: "float_info".to_string(),
                    dict: Box::new(str_map_to_typedict(HashMap::new())),
                    bases: vec![],
                    mro: vec![],
                }),
                dict: float_info_dict,
            }),
        );
    }
    {
        // sys._jit — CPython 3.13+'s experimental copy-and-patch JIT
        // introspection object (`sys._jit.is_enabled()`/`is_active()`).
        // Unrelated to this interpreter's own optional Cranelift `jit`
        // Cargo feature; either way the correct answer for test purposes
        // is "not enabled". Real trigger: `test.support`'s own
        // `_JIT_ENABLED = sys._jit.is_enabled()`.
        let mut jit_dict = AttrMap::new();
        jit_dict.insert_str(
            "is_enabled",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "is_enabled".to_string(),
                func: |_args| Ok(py_bool(false)),
            }),
        );
        jit_dict.insert_str(
            "is_active",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "is_active".to_string(),
                func: |_args| Ok(py_bool(false)),
            }),
        );
        d.insert_str(
            "_jit",
            PyObjectRef::new(PyObject::Instance {
                typ: PyObjectRef::new(PyObject::Type {
                    name: "_jit".to_string(),
                    dict: Box::new(str_map_to_typedict(HashMap::new())),
                    bases: vec![],
                    mro: vec![],
                }),
                dict: jit_dict,
            }),
        );
    }
    {
        // sys.monitoring — PEP 669 (CPython 3.12+) monitoring API stub.
        // `Lib/bdb.py` (3.14) does `E = sys.monitoring.events` at import
        // time, so the attribute must exist for `import bdb` / test_bdb.
        // Real event delivery is not implemented (no-op); Bdb defaults to
        // backend='settrace' which does not need monitoring.
        // Event values are 1<<index matching CPython's monitoring.h.
        let mut events_dict = AttrMap::new();
        events_dict.insert_str("PY_START", py_int(1));
        events_dict.insert_str("PY_RESUME", py_int(2));
        events_dict.insert_str("PY_RETURN", py_int(4));
        events_dict.insert_str("PY_YIELD", py_int(8));
        events_dict.insert_str("CALL", py_int(16));
        events_dict.insert_str("LINE", py_int(32));
        events_dict.insert_str("INSTRUCTION", py_int(64));
        events_dict.insert_str("JUMP", py_int(128));
        events_dict.insert_str("BRANCH_LEFT", py_int(256));
        events_dict.insert_str("BRANCH_RIGHT", py_int(512));
        events_dict.insert_str("STOP_ITERATION", py_int(1024));
        events_dict.insert_str("RAISE", py_int(2048));
        events_dict.insert_str("EXCEPTION_HANDLED", py_int(4096));
        events_dict.insert_str("PY_UNWIND", py_int(8192));
        events_dict.insert_str("PY_THROW", py_int(16384));
        events_dict.insert_str("RERAISE", py_int(32768));
        events_dict.insert_str("C_RETURN", py_int(65536));
        events_dict.insert_str("C_RAISE", py_int(131072));
        events_dict.insert_str("BRANCH", py_int(262144));
        events_dict.insert_str("NO_EVENTS", py_int(0));
        let events_obj = PyObjectRef::new(PyObject::Instance {
            typ: PyObjectRef::new(PyObject::Type {
                name: "events".to_string(),
                dict: Box::new(str_map_to_typedict(HashMap::new())),
                bases: vec![],
                mro: vec![],
            }),
            dict: events_dict,
        });
        let mut mon_dict = AttrMap::new();
        mon_dict.insert_str("DEBUGGER_ID", py_int(0));
        mon_dict.insert_str("COVERAGE_ID", py_int(1));
        mon_dict.insert_str("PROFILER_ID", py_int(2));
        mon_dict.insert_str("OPTIMIZER_ID", py_int(5));
        mon_dict.insert_str("events", events_obj);
        // DISABLE / MISSING singletons — unique instances for identity checks.
        let make_sentinel = |name: &str| {
            PyObjectRef::new(PyObject::Instance {
                typ: PyObjectRef::new(PyObject::Type {
                    name: name.to_string(),
                    dict: Box::new(str_map_to_typedict(HashMap::new())),
                    bases: vec![],
                    mro: vec![],
                }),
                dict: AttrMap::new(),
            })
        };
        mon_dict.insert_str("DISABLE", make_sentinel("DISABLE"));
        mon_dict.insert_str("MISSING", make_sentinel("MISSING"));
        mon_dict.insert_str(
            "use_tool_id",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "use_tool_id".to_string(),
                func: |_args| Ok(py_none()),
            }),
        );
        mon_dict.insert_str(
            "free_tool_id",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "free_tool_id".to_string(),
                func: |_args| Ok(py_none()),
            }),
        );
        mon_dict.insert_str(
            "get_tool",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "get_tool".to_string(),
                func: |_args| Ok(py_none()),
            }),
        );
        mon_dict.insert_str(
            "clear_tool_id",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "clear_tool_id".to_string(),
                func: |_args| Ok(py_none()),
            }),
        );
        mon_dict.insert_str(
            "set_events",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "set_events".to_string(),
                func: |_args| Ok(py_none()),
            }),
        );
        mon_dict.insert_str(
            "get_events",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "get_events".to_string(),
                func: |_args| Ok(py_int(0)),
            }),
        );
        mon_dict.insert_str(
            "set_local_events",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "set_local_events".to_string(),
                func: |_args| Ok(py_none()),
            }),
        );
        mon_dict.insert_str(
            "get_local_events",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "get_local_events".to_string(),
                func: |_args| Ok(py_int(0)),
            }),
        );
        mon_dict.insert_str(
            "register_callback",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "register_callback".to_string(),
                func: |_args| Ok(py_none()),
            }),
        );
        mon_dict.insert_str(
            "restart_events",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "restart_events".to_string(),
                func: |_args| Ok(py_none()),
            }),
        );
        d.insert_str(
            "monitoring",
            PyObjectRef::new(PyObject::Instance {
                typ: PyObjectRef::new(PyObject::Type {
                    name: "monitoring".to_string(),
                    dict: Box::new(str_map_to_typedict(HashMap::new())),
                    bases: vec![],
                    mro: vec![],
                }),
                dict: mon_dict,
            }),
        );
    }
    d.insert_str(
        "stdin",
        PyObjectRef::new(PyObject::File {
            file: std::rc::Rc::new(std::cell::RefCell::new(
                dup_std_fd(0).unwrap_or_else(|_| std::fs::File::open("/dev/null").unwrap()),
            )),
            name: "<stdin>".to_string(),
            binary: false,
            pending: std::rc::Rc::new(std::cell::RefCell::new(Vec::new())),
            closed: std::rc::Rc::new(std::cell::RefCell::new(false)),
            encoding: None,
        }),
    );
    d.insert_str(
        "stdout",
        PyObjectRef::new(PyObject::File {
            file: std::rc::Rc::new(std::cell::RefCell::new(
                dup_std_fd(1).unwrap_or_else(|_| std::fs::File::create("/dev/null").unwrap()),
            )),
            name: "<stdout>".to_string(),
            binary: false,
            pending: std::rc::Rc::new(std::cell::RefCell::new(Vec::new())),
            closed: std::rc::Rc::new(std::cell::RefCell::new(false)),
            encoding: None,
        }),
    );
    d.insert_str(
        "stderr",
        PyObjectRef::new(PyObject::File {
            file: std::rc::Rc::new(std::cell::RefCell::new(
                dup_std_fd(2).unwrap_or_else(|_| std::fs::File::create("/dev/null").unwrap()),
            )),
            name: "<stderr>".to_string(),
            binary: false,
            pending: std::rc::Rc::new(std::cell::RefCell::new(Vec::new())),
            closed: std::rc::Rc::new(std::cell::RefCell::new(false)),
            encoding: None,
        }),
    );
    d.insert_str("platform", py_str(std::env::consts::OS));
    // sys.__stdout__/__stderr__/__stdin__ — saved originals for
    // capture/restore patterns (test_calendar, etc.)
    {
        let std_in = d.get_str("stdin").cloned().unwrap();
        let std_out = d.get_str("stdout").cloned().unwrap();
        let std_err = d.get_str("stderr").cloned().unwrap();
        d.insert_str("__stdin__", std_in);
        d.insert_str("__stdout__", std_out);
        d.insert_str("__stderr__", std_err);
    }
    // sys.implementation — CPython returns a namespace with name, cache_tag, etc.
    // Use "rustpython" so `support.cpython_only` correctly skips CPython-specific
    // tests (e.g. test_deque::test_sizeof which checks exact CPython struct layout).
    {
        let mut imp_dict = HashMap::new();
        imp_dict.insert_str("name", py_str("rustpython"));
        imp_dict.insert_str("cache_tag", py_str("rustpython-314"));
        imp_dict.insert_str("hexversion", py_int(50987248)); // 0x030A0000
        imp_dict.insert_str("_multi_threaded", py_bool(true));
        d.insert_str("implementation", create_module("implementation", imp_dict));
    }
    d.insert_str(
        "byteorder",
        py_str(if cfg!(target_endian = "little") {
            "little"
        } else {
            "big"
        }),
    );
    d.insert_str("platlibdir", py_str("lib"));
    // `sys.builtin_module_names` — missing entirely (AttributeError),
    // breaking any code that distinguishes "compiled into the interpreter"
    // modules from filesystem-loaded ones (real trigger: `modulefinder.py`'s
    // `import_hook`, vendored verbatim from CPython, checks
    // `name in sys.builtin_module_names` for every import). Real CPython's
    // own set varies by build; this lists the modules genuinely
    // implemented natively in Rust (not loaded from `Lib/`) rather than a
    // hand-copied CPython list, so it can't drift from what this
    // interpreter actually provides without a filesystem import.
    d.insert_str(
        "builtin_module_names",
        py_tuple(
            [
                "sys",
                "builtins",
                "_abc",
                "_codecs",
                "_collections",
                "_functools",
                "_io",
                "_warnings",
                "_thread",
                "_imp",
                "_ast",
                "gc",
                "itertools",
                "marshal",
                "math",
                "time",
                "posix",
                "errno",
            ]
            .iter()
            .map(|s| py_str(s))
            .collect(),
        ),
    );
    d.insert_str("maxsize", py_int(i64::MAX));
    d.insert_str("maxunicode", py_int(1114111));
    d.insert_str("api_version", py_int(1013));
    d.insert_str(
        "executable",
        py_str(
            &std::env::current_exe()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default(),
        ),
    );
    // `sys._base_executable` — CPython's real, non-venv interpreter path,
    // used internally by `venv`/`ensurepip`/`site` (e.g. `EnvBuilder.
    // ensure_directories()` reads it directly) to find the real interpreter
    // to symlink/copy into a new venv, distinct from `sys.executable` (which
    // — like `sys.prefix` above — follows whatever venv is CURRENTLY
    // active). Was missing entirely: any of that tooling raised
    // `AttributeError: module 'sys' has no attribute '_base_executable'`
    // instead of building a venv. Same value as `executable` here since
    // this interpreter isn't itself venv-relocatable the way CPython is.
    d.insert_str(
        "_base_executable",
        py_str(
            &std::env::current_exe()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default(),
        ),
    );
    // `sys._framework` — real CPython always defines this (see
    // `sysmodule.c`'s `SET_SYS_FROM_STRING("_framework", _PYTHONFRAMEWORK)`),
    // even off macOS, as an empty string. Missing entirely here caused a
    // bare AttributeError in code that reads it unconditionally (real
    // trigger: `test_sysconfig.py`'s `setUp`/`tearDown` unconditionally
    // save/restore `sys._framework`).
    d.insert_str("_framework", py_str(""));
    // Detect virtual environment (uv, venv, virtualenv, conda, poetry, pixi)
    let venv_path = std::env::var("VIRTUAL_ENV")
        .ok()
        .or_else(|| std::env::var("CONDA_PREFIX").ok())
        .or_else(|| {
            // Poetry: POETRY_ACTIVE is set when inside a poetry shell
            // Also check POETRY_VIRTUAL_ENV which poetry sets explicitly
            if std::env::var("POETRY_ACTIVE").is_ok() {
                std::env::var("POETRY_VIRTUAL_ENV").ok()
            } else {
                None
            }
        })
        .or_else(|| {
            // Pixi environments
            std::env::var("PIXI_IN_SHELL")
                .ok()
                .and_then(|_| std::env::var("PIXI_PROJECT_ROOT").ok())
        })
        .or_else(|| {
            // Also look for .venv in CWD
            let cwd = std::env::current_dir().ok()?;
            let dot_venv = cwd.join(".venv");
            if dot_venv.is_dir() {
                Some(dot_venv.to_string_lossy().to_string())
            } else {
                None
            }
        });
    let (prefix, exec_prefix) = if let Some(ref venv) = venv_path {
        (venv.clone(), venv.clone())
    } else {
        ("/usr".to_string(), "/usr".to_string())
    };
    d.insert_str("prefix", py_str(&prefix));
    d.insert_str("exec_prefix", py_str(&exec_prefix));
    // `sys.base_prefix`/`base_exec_prefix` — real CPython's own venv-
    // detection idiom (`sys.prefix != sys.base_prefix`) needs these to be
    // the REAL, non-venv installation prefix regardless of whether a venv
    // is currently active — unlike `prefix`/`exec_prefix` above (which
    // deliberately follow the active venv). Was missing entirely, meaning
    // any code doing this exact "am I in a venv" check (a common pattern
    // — `pip`, `venv` itself, build tooling) raised `AttributeError`
    // instead of getting a straight answer.
    d.insert_str("base_prefix", py_str("/usr"));
    d.insert_str("base_exec_prefix", py_str("/usr"));
    d.insert_str("winver", py_str("3.12"));
    // sys.exc_info() — returns current exception info from VM. Real logic
    // lives in `sys_exc_info_builtin` (a real top-level fn, not inlined
    // here) so `vm.rs`'s `call_function` can recognize and special-case it
    // by pointer identity — see the fix there for why `with_vm_mut` alone
    // is unsafe.
    sys_func!("getfilesystemencoding", |_args| Ok(py_str("utf-8")));
    sys_func!("getfilesystemencodeerrors", |_args| Ok(py_str(
        "surrogateescape"
    )));
    sys_func!("getdefaultencoding", |_args| Ok(py_str("utf-8")));
    sys_func!("exc_info", sys_exc_info_builtin);
    // `sys.exception()` (3.11+) — returns just the currently-handled
    // exception INSTANCE (or `None`), equivalent to `sys.exc_info()[1]`.
    // Missing entirely (`AttributeError`) broke `Lib/contextlib.py`'s own
    // `_GeneratorContextManagerBase`/`_AsyncGeneratorContextManagerBase`
    // internals (`frame_exc = sys.exception()`) — a module imported
    // pervasively, so this affected many otherwise-unrelated test files the
    // moment they merely imported something that pulls in `contextlib`.
    sys_func!("exception", sys_exception_builtin);
    sys_func!("getrecursionlimit", sys_getrecursionlimit_builtin);
    sys_func!("setrecursionlimit", sys_setrecursionlimit_builtin);
    // `sys.get_recursion_depth()` — returns current recursion depth.
    // test_xml_etree and test_io both call this.
    sys_func!("get_recursion_depth", |_args| {
        Ok(py_int(0)) // simplified: always return 0
    });
    sys_func!("settrace", sys_settrace_builtin);
    sys_func!("gettrace", sys_gettrace_builtin);
    sys_func!("setprofile", sys_setprofile_builtin);
    sys_func!("getprofile", sys_getprofile_builtin);
    sys_func!("_getframe", sys_getframe_builtin);
    sys_func!("is_remote_debug_enabled", |_args| Ok(py_bool(false)));
    sys_func!("get_int_max_str_digits", |_| {
        Ok(py_int(crate::object::INT_MAX_STR_DIGITS.with(|d| d.get())))
    });
    sys_func!("set_int_max_str_digits", |args| {
        let val = if args.len() >= 1 {
            args[0].as_i64().unwrap_or(4300)
        } else {
            4300
        };
        crate::object::INT_MAX_STR_DIGITS.with(|d| d.set(val));
        Ok(py_none())
    });
    // `sys.getsizeof(obj)` — was missing entirely. Real CPython reports the
    // actual C-level memory footprint of `obj`, which has no equivalent
    // meaning against this interpreter's own, completely different object
    // representation — so this is a deliberate APPROXIMATION (rough,
    // per-type base size + a per-element estimate for containers) good
    // enough for code that just checks `getsizeof(x) > 0` or compares
    // relative sizes between two objects of the same type, not for code
    // asserting on an exact byte count (which would be fragile even
    // between two real CPython builds/versions anyway).
    // `sys.getrefcount(obj)` — was missing entirely. See `PyObjectRef::
    // strong_count`'s own doc comment for why this reports `Rc::strong_count`
    // (a real, meaningful delta signal) rather than attempting to match
    // CPython's absolute refcount convention, which has no equivalent here.
    // The `+1` matches real CPython's own documented behavior: the argument
    // passed to `getrefcount` itself holds one additional temporary
    // reference (the call's own argument slot) beyond whatever the caller's
    // other variables hold.
    sys_func!("getrefcount", |args| {
        if args.is_empty() {
            return Err(PyError::type_error(
                "getrefcount() takes exactly one argument (0 given)",
            ));
        }
        Ok(py_int(args[0].strong_count() as i64 + 1))
    });
    sys_func!("getsizeof", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("getsizeof() takes at least 1 argument"));
        }
        let size = match &*args[0].borrow() {
            PyObject::None => 16,
            PyObject::Bool(_) => 28,
            PyObject::Int(_) => 28,
            PyObject::Float(_) => 24,
            PyObject::Str(s) => 49 + s.len() as i64,
            PyObject::Bytes(b) => 33 + b.len() as i64,
            PyObject::ByteArray(b) => 56 + b.len() as i64,
            PyObject::List(v) => 56 + 8 * v.len() as i64,
            PyObject::Tuple(v) => 40 + 8 * v.len() as i64,
            PyObject::Dict(d) => 64 + 32 * d.len() as i64,
            PyObject::Set(s) | PyObject::FrozenSet(s) => 216 + 32 * s.len() as i64,
            _ => 48,
        };
        Ok(py_int(size))
    });
    d
}
