use crate::object::*;
use std::collections::HashMap;
use std::sync::atomic::AtomicI64;
use num_traits::{Signed, ToPrimitive};
use std::rc::Rc;

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
            if offset >= 1 && !text.is_empty() {
                let indent = "    "; // 4-space indent
                let text_stripped = text.trim_end_matches('\n');
                let caret_len = if let Some(end) = end_offset {
                    // Use end_offset for caret length
                    let start = (offset - 1) as usize;
                    let end = (end - 1) as usize;
                    std::cmp::max(1, end - start + 1)
                } else {
                    // Fallback: caret extends to end of line
                    std::cmp::max(1, text_stripped.len() - offset as usize + 1)
                };
                let spaces = (offset - 1) as usize;
                let caret_line = format!("{}{}{}\n", indent, " ".repeat(spaces), "^".repeat(caret_len));
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
        let code = if args.len() > 0 {
            match &*args[0].borrow() {
                PyObject::Int(i) => i.to_i64().unwrap_or(0) as i32,
                _ => 1,
            }
        } else {
            0
        };
        Err(PyError::SystemExit(code))
    });
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
    d.insert_str("version", py_str("3.12.0 (RustPython 0.1.0)"));
    d.insert_str(
        "version_info",
        py_tuple(vec![py_int(3), py_int(12), py_int(0)]),
    );
    d.insert_str("float_repr_style", py_str("short"));
    d.insert_str("hexversion", py_int(0x030c0000));
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
    d.insert_str(
        "stdin",
        PyObjectRef::new(PyObject::File {
            file: std::rc::Rc::new(std::cell::RefCell::new(
                dup_std_fd(0).unwrap_or_else(|_| std::fs::File::open("/dev/null").unwrap()),
            )),
            name: "<stdin>".to_string(),
            binary: false,
            pending: std::rc::Rc::new(std::cell::RefCell::new(Vec::new())),
            closed: false,
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
            closed: false,
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
            closed: false,
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
    sys_func!("settrace", sys_settrace_builtin);
    sys_func!("gettrace", sys_gettrace_builtin);
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
