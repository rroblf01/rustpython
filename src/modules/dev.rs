use crate::bytecode::{needs_arg, CodeObject};
use crate::interner;
use crate::object::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;


mod pdb;
pub use pdb::*;
mod traceback;
pub use traceback::*;
mod opcode;
pub use opcode::*;
mod language;
pub use language::*;
mod inspect;
pub use inspect::*;
mod io;
pub use io::*;

pub fn create_profile_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! prof_func {
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

    prof_func!("run", |args| {
        if args.is_empty() {
            return Err(PyError::type_error(
                "run() missing required argument (statement)",
            ));
        }
        let cmd = args[0].str();
        let _ = crate::object::with_vm_mut(|vm| {
            let mut parser = crate::parser::Parser::new(&cmd);
            if let Ok(program) = parser.parse_program() {
                let mut compiler = crate::compiler::Compiler::new();
                if let Ok(code) = compiler.compile(&program, "<profile>") {
                    let _ = vm.exec_code(code, None);
                }
            }
        });
        Ok(py_none())
    });

    prof_func!("runctx", |args| {
        if args.len() < 3 {
            return Err(PyError::type_error(
                "runctx() requires 3 arguments (statement, globals, locals)",
            ));
        }
        let cmd = args[0].str();
        let _globals = &args[1];
        let _locals = &args[2];
        let _ = crate::object::with_vm_mut(|vm| {
            let mut parser = crate::parser::Parser::new(&cmd);
            if let Ok(program) = parser.parse_program() {
                let mut compiler = crate::compiler::Compiler::new();
                if let Ok(code) = compiler.compile(&program, "<profile>") {
                    let _ = vm.exec_code(code, None);
                }
            }
        });
        Ok(py_none())
    });

    // Profiler stub class
    prof_func!("Profile", |_args| {
        let mut inst_dict = AttrMap::new();
        inst_dict.insert_str(
            "enable",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "enable".to_string(),
                func: |_| Ok(py_none()),
            }),
        );
        inst_dict.insert_str(
            "disable",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "disable".to_string(),
                func: |_| Ok(py_none()),
            }),
        );
        inst_dict.insert_str(
            "create_stats",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "create_stats".to_string(),
                func: |_| Ok(py_none()),
            }),
        );
        inst_dict.insert_str(
            "print_stats",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "print_stats".to_string(),
                func: |_| Ok(py_none()),
            }),
        );
        inst_dict.insert_str(
            "dump_stats",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "dump_stats".to_string(),
                func: |_| Ok(py_none()),
            }),
        );
        Ok(PyObjectRef::new(PyObject::Instance {
            typ: py_str("Profile"),
            dict: inst_dict,
        }))
    });

    d
}

// ─── cProfile module ───────────────────────────────────────────────────────

pub fn create_cprofile_dict() -> HashMap<String, PyObjectRef> {
    let mut d = create_profile_dict();
    d.insert_str("__name__", py_str("cProfile"));
    d
}

// ─── resource module ──────────────────────────────────────────────────────

pub fn create_resource_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! res_func {
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

    // Resource usage constants (POSIX standard)
    d.insert_str("RUSAGE_SELF", py_int(0));
    d.insert_str("RUSAGE_CHILDREN", py_int(-1));
    d.insert_str("RUSAGE_BOTH", py_int(-2));
    d.insert_str("RUSAGE_THREAD", py_int(1));

    // Priority constants
    d.insert_str("PRIO_PROCESS", py_int(0));
    d.insert_str("PRIO_PGRP", py_int(1));
    d.insert_str("PRIO_USER", py_int(2));

    // RLIMIT constants (common ones)
    d.insert_str("RLIMIT_CPU", py_int(0));
    d.insert_str("RLIMIT_FSIZE", py_int(1));
    d.insert_str("RLIMIT_DATA", py_int(2));
    d.insert_str("RLIMIT_STACK", py_int(3));
    d.insert_str("RLIMIT_CORE", py_int(4));
    d.insert_str("RLIMIT_NOFILE", py_int(7));
    d.insert_str("RLIMIT_AS", py_int(9));

    res_func!("getrusage", |_args| {
        let mut result_dict = AttrMap::new();
        let zero = py_int(0);
        result_dict.insert_str("ru_utime", py_float(0.0));
        result_dict.insert_str("ru_stime", py_float(0.0));
        result_dict.insert_str("ru_maxrss", zero.clone());
        result_dict.insert_str("ru_ixrss", zero.clone());
        result_dict.insert_str("ru_idrss", zero.clone());
        result_dict.insert_str("ru_isrss", zero.clone());
        result_dict.insert_str("ru_minflt", zero.clone());
        result_dict.insert_str("ru_majflt", zero.clone());
        result_dict.insert_str("ru_nswap", zero.clone());
        result_dict.insert_str("ru_inblock", zero.clone());
        result_dict.insert_str("ru_oublock", zero.clone());
        result_dict.insert_str("ru_msgsnd", zero.clone());
        result_dict.insert_str("ru_msgrcv", zero.clone());
        result_dict.insert_str("ru_nsignals", zero.clone());
        result_dict.insert_str("ru_nvcsw", zero.clone());
        result_dict.insert_str("ru_nivcsw", zero.clone());
        Ok(PyObjectRef::new(PyObject::Instance {
            typ: py_str("struct_rusage"),
            dict: result_dict,
        }))
    });

    res_func!("getpagesize", |_| { Ok(py_int(4096)) });

    res_func!("getrlimit", |_args| {
        // Return (soft, hard) as tuple with large defaults
        Ok(py_tuple(vec![py_int(999999), py_int(999999)]))
    });

    res_func!("setrlimit", |_args| { Ok(py_none()) });

    d
}

// ─── trace module ─────────────────────────────────────────────────────────

pub fn create_trace_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! trace_func {
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

    trace_func!("Trace", |_args| {
        let mut inst_dict = AttrMap::new();
        inst_dict.insert_str(
            "run",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "run".to_string(),
                func: |args| {
                    let cmd = if !args.is_empty() {
                        args[0].str()
                    } else {
                        String::new()
                    };
                    let _ = crate::object::with_vm_mut(|vm| {
                        let mut parser = crate::parser::Parser::new(&cmd);
                        if let Ok(program) = parser.parse_program() {
                            let mut compiler = crate::compiler::Compiler::new();
                            if let Ok(code) = compiler.compile(&program, "<trace>") {
                                let _ = vm.exec_code(code, None);
                            }
                        }
                    });
                    Ok(py_none())
                },
            }),
        );
        inst_dict.insert_str(
            "runctx",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "runctx".to_string(),
                func: |_| Ok(py_none()),
            }),
        );
        inst_dict.insert_str(
            "results",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "results".to_string(),
                func: |_| Ok(py_none()),
            }),
        );
        Ok(PyObjectRef::new(PyObject::Instance {
            typ: py_str("Trace"),
            dict: inst_dict,
        }))
    });

    // Coverage results class
    trace_func!("CoverageResults", |_args| {
        let mut inst_dict = AttrMap::new();
        inst_dict.insert_str(
            "write_results",
            PyObjectRef::new(PyObject::BuiltinFunction {
                name: "write_results".to_string(),
                func: |_| Ok(py_none()),
            }),
        );
        Ok(PyObjectRef::new(PyObject::Instance {
            typ: py_str("CoverageResults"),
            dict: inst_dict,
        }))
    });

    d
}

/// Native _warnings module — CPython C extension replacement

pub fn create_marshal_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! m_func {
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
    m_func!("loads", |args| {
        let data = args
            .first()
            .ok_or_else(|| PyError::type_error("loads() missing required argument 'bytes'"))?;
        if let PyObject::Bytes(b) = &*data.borrow() {
            if b.is_empty() {
                return Err(PyError::Exception(
                    "EOFError".to_string(),
                    PyObjectRef::new(PyObject::Exception {
                        typ: "EOFError".to_string(),
                        args: vec![],
                        cause: None,
                        suppress_context: false,
                        context: None,
                        traceback: None,
                        extra: None,
                    }),
                ));
            }
            // Decode the minimal round-trip markers emitted by dumps.
            match b[0] {
                0x54 => return Ok(py_bool(true)),
                0x46 => return Ok(py_bool(false)),
                b'i' => {
                    let s = String::from_utf8_lossy(&b[1..])
                        .trim_end_matches('\0')
                        .to_string();
                    if let Ok(n) = s.parse::<num_bigint::BigInt>() {
                        return Ok(py_int(n));
                    }
                }
                b'g' => {
                    if b.len() >= 9 {
                        let mut arr = [0u8; 8];
                        arr.copy_from_slice(&b[1..9]);
                        return Ok(py_float(f64::from_bits(u64::from_le_bytes(arr))));
                    }
                }
                b's' => {
                    if b.len() >= 5 {
                        let len = i32::from_le_bytes([b[1], b[2], b[3], b[4]]) as usize;
                        if b.len() >= 5 + len {
                            return Ok(py_str(&String::from_utf8_lossy(&b[5..5 + len])));
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(data.clone())
    });
    m_func!("intern", |args| {
        let obj = args
            .first()
            .ok_or_else(|| PyError::type_error("intern() missing required argument 'string'"))?;
        Ok(obj.clone())
    });
    m_func!("dumps", |args| {
        let obj = args
            .first()
            .ok_or_else(|| PyError::type_error("dumps() missing required argument 'value'"))?;
        // Minimal but real: round-trip bool/int/float/str through marshal's
        // own marker bytes so `marshal.loads(marshal.dumps(x)) == x`
        // (test_bool::test_marshal exercises True).
        let bytes: Vec<u8> = match &*obj.borrow() {
            PyObject::Bool(b) => vec![if *b { 0x54u8 } else { 0x46 }],
            PyObject::Int(i) => {
                let mut v = vec![b'i'];
                v.extend_from_slice(i.to_string().as_bytes());
                v.push(0);
                v
            }
            PyObject::Float(f) => {
                let mut v = vec![b'g'];
                v.extend_from_slice(&f.to_bits().to_le_bytes());
                v
            }
            PyObject::Str(s) => {
                let mut v = vec![b's'];
                v.extend_from_slice(&(s.len() as i32).to_le_bytes());
                v.extend_from_slice(s.as_bytes());
                v
            }
            _ => {
                return Err(PyError::type_error(format!(
                    "cannot marshal {} object",
                    obj.borrow().type_name()
                )))
            }
        };
        Ok(PyObjectRef::imm(PyObject::Bytes(bytes)))
    });
    d
}

pub fn create_imp_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! imp_func {
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

    imp_func!("acquire_lock", |_| Ok(py_none()));
    imp_func!("release_lock", |_| Ok(py_none()));
    imp_func!("lock_held", |_| Ok(py_bool(false)));
    imp_func!("is_frozen", |_| Ok(py_bool(false)));
    imp_func!("is_builtin", |_| Ok(py_bool(false)));
    imp_func!("is_frozen_package", |_| Ok(py_bool(false)));
    imp_func!("find_frozen", |_| Err(PyError::ImportError(
        "frozen modules not supported".to_string()
    )));
    imp_func!("init_frozen", |_| Ok(py_none()));
    imp_func!("get_frozen_object", |_| Err(PyError::ImportError(
        "frozen modules not supported".to_string()
    )));
    imp_func!("create_builtin", |args| {
        // Return a new module object for builtin modules
        let spec = if !args.is_empty() {
            args[0].borrow()
        } else {
            return Err(PyError::type_error("create_builtin requires spec"));
        };
        let name = spec
            .get_attribute("name")
            .ok()
            .map(|n| n.str())
            .unwrap_or_else(|| "unknown".to_string());
        Ok(create_module(&name, HashMap::new()))
    });
    imp_func!("exec_builtin", |_args| {
        // No-op: module is already registered
        Ok(py_none())
    });
    imp_func!("create_dynamic", |_| Err(PyError::ImportError(
        "dynamic extensions not supported".to_string()
    )));
    imp_func!("exec_dynamic", |_| Err(PyError::ImportError(
        "dynamic extensions not supported".to_string()
    )));

    imp_func!("extension_suffixes", |_| {
        let arch = if cfg!(target_os = "linux") {
            "x86_64-linux-gnu"
        } else if cfg!(target_os = "macos") {
            "darwin"
        } else {
            "win-amd64"
        };
        Ok(py_list(vec![
            py_str(&format!(".cpython-313-{}.so", arch)),
            py_str(".abi3.so"),
            py_str(".so"),
        ]))
    });

    imp_func!("source_hash", |_| Ok(PyObjectRef::imm(PyObject::Bytes(
        vec![0u8; 8]
    ))));
    imp_func!("_fix_co_filename", |_| Ok(py_none()));

    d.insert_str("check_hash_based_pycs", py_str("never"));
    d.insert_str("_frozen_module_names", py_list(vec![]));
    // Both were bare `py_none()` placeholders — not callable at all — which
    // broke `test.support.import_helper.frozen_modules()`/
    // `multi_interp_extensions_check()` (both real CPython context managers
    // wrapping a call to one of these) with `TypeError: 'NoneType' object is
    // not callable`, for any test file using `import_fresh_module`/`CleanImport`
    // (an extremely common test-infrastructure idiom — real trigger: 10+
    // corpus files hit this identical symptom via `import_helper`).
    imp_func!("_override_frozen_modules_for_tests", |_| Ok(py_none()));
    imp_func!("_override_multi_interp_extensions_check", |_| Ok(py_none()));

    d
}

/// Native _io module — CPython C extension replacement


pub(crate) fn io_get_raw(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::attribute_error("no underlying raw object"));
    }
    if let PyObject::Instance { dict, .. } = &*args[0].borrow() {
        if let Some(r) = dict.get("_raw") {
            return Ok(r.clone());
        }
    }
    Err(PyError::attribute_error("no underlying raw object"))
}
pub(crate) fn io_ensure_open(args: &[PyObjectRef]) -> PyResult<()> {
    if args.is_empty() {
        return Ok(());
    }
    // Only buffered wrapper instances have _closed. Use try_borrow: the
    // caller may already hold a borrow (e.g. readinto called from within
    // another operation on the same object).
    if let PyObjectRef::Mut(rc) | PyObjectRef::Imm(rc) = &args[0] {
        if let Ok(b) = rc.try_borrow() {
            if let PyObject::Instance { dict, .. } = &*b {
                if let Some(c) = dict.get("_closed") {
                    if c.truthy() {
                        return Err(PyError::value_error(
                            "I/O operation on closed file",
                        ));
                    }
                }
            }
        }
    }
    Ok(())
}

/// Build one of the io delegation wrapper classes (BufferedReader etc.):
/// stores `_raw` on the instance and forwards every I/O method through it.
#[macro_export]
macro_rules! buffered_class {
    ($name:expr, $buf:expr) => {{
        let mut td: HashMap<String, PyObjectRef> = HashMap::new();
        let bf = |name: &'static str, f: crate::object::BuiltinFunc| {
            PyObjectRef::new(PyObject::BuiltinFunction { name: name.to_string(), func: f })
        };
        td.insert("__init__".into(), bf("__init__", |args| {
            if args.len() < 2 {
                return Err(PyError::type_error("missing required argument 'raw'"));
            }
            if let PyObject::Instance { dict, .. } = &mut *args[0].borrow_mut() {
                dict.insert_str("_raw", args[1].clone());
                dict.insert_str("_closed", py_bool(false));
            }
            Ok(py_none())
        }));
        macro_rules! read_arm {
            ($m:literal) => { bf($m, |args: &[PyObjectRef]| {
                crate::modules::dev::io_ensure_open(args)?;
                let raw = crate::modules::dev::io_get_raw(args)?;
                let mut margs: Vec<PyObjectRef> = Vec::new();
                if let Some(n) = args.get(1) { margs.push(n.clone()); }
                match crate::object::with_vm_mut(|vm| {
                    crate::object::call_method_rebound(vm, &raw, $m, margs)
                }) {
                    Ok(Ok(v)) => Ok(v),
                    Ok(Err(e)) => Err(e),
                    Err(e) => Err(e),
                }
            })};
        }
        td.insert("read".into(), read_arm!("read"));
        td.insert("read1".into(), read_arm!("read1"));
        td.insert("readline".into(), read_arm!("readline"));
        td.insert("readlines".into(), bf("readlines", |args: &[PyObjectRef]| {
            crate::modules::dev::io_ensure_open(args)?;
            let raw = crate::modules::dev::io_get_raw(args)?;
            crate::object::with_vm_mut(|vm| crate::object::call_method_rebound(vm, &raw, "readlines", vec![]))?
        }));
        td.insert("readinto".into(), bf("readinto", |args: &[PyObjectRef]| {
            crate::modules::dev::io_ensure_open(args)?;
            // Validate argument is a writable buffer BEFORE forwarding.
            if args.len() > 1 {
                match &*args[1].borrow() {
                    PyObject::ByteArray(_) | PyObject::MemoryView { .. } => {}
                    _ => return Err(PyError::type_error(
                        "'int' object does not support the buffer interface",
                    )),
                }
            }
            let raw = crate::modules::dev::io_get_raw(args)?;
            let b = args.get(1).cloned().unwrap_or_else(py_none);
            crate::object::with_vm_mut(|vm| crate::object::call_method_rebound(vm, &raw, "readinto", vec![b]))?
        }));
        td.insert("write".into(), bf("write", |args: &[PyObjectRef]| {
            crate::modules::dev::io_ensure_open(args)?;
            let data = args.get(1).cloned().ok_or_else(|| PyError::type_error("write missing data"))?;
            let n = match &*data.borrow() {
                PyObject::Bytes(b) => b.len() as i64,
                PyObject::Str(s2) => s2.chars().count() as i64,
                _ => 0,
            };
            let raw = crate::modules::dev::io_get_raw(args)?;
            let r = crate::object::with_vm_mut(|vm| crate::object::call_method_rebound(vm, &raw, "write", vec![data]))??;
            if matches!(&*r.borrow(), PyObject::None) {
                Ok(py_int(n))
            } else { Ok(r) }
        }));
        td.insert("seek".into(), bf("seek", |args: &[PyObjectRef]| {
            crate::modules::dev::io_ensure_open(args)?;
            let raw = crate::modules::dev::io_get_raw(args)?;
            let pos = args.get(1).cloned().unwrap_or_else(|| py_int(0));
            let wh = args.get(2).cloned().unwrap_or_else(|| py_int(0));
            crate::object::with_vm_mut(|vm| crate::object::call_method_rebound(vm, &raw, "seek", vec![pos, wh]))?
        }));
        td.insert("tell".into(), bf("tell", |args: &[PyObjectRef]| {
            crate::modules::dev::io_ensure_open(args)?;
            let raw = crate::modules::dev::io_get_raw(args)?;
            crate::object::with_vm_mut(|vm| crate::object::call_method_rebound(vm, &raw, "tell", vec![]))?
        }));
        td.insert("truncate".into(), bf("truncate", |args: &[PyObjectRef]| {
            crate::modules::dev::io_ensure_open(args)?;
            let raw = crate::modules::dev::io_get_raw(args)?;
            let pos = args.get(1).cloned().unwrap_or_else(py_none);
            crate::object::with_vm_mut(|vm| crate::object::call_method_rebound(vm, &raw, "truncate", vec![pos]))?
        }));
        td.insert("flush".into(), bf("flush", |args: &[PyObjectRef]| {
            crate::modules::dev::io_ensure_open(args)?;
            let raw = crate::modules::dev::io_get_raw(args)?;
            crate::object::with_vm_mut(|vm| crate::object::call_method_rebound(vm, &raw, "flush", vec![]))?
        }));
        td.insert("close".into(), bf("close", |args: &[PyObjectRef]| {
            let already = args[0].borrow().get_attribute("_closed").ok().map(|v| v.truthy()).unwrap_or(false);
            if !already {
                let raw = crate::modules::dev::io_get_raw(args)?;
                let _ = crate::object::with_vm_mut(|vm| crate::object::call_method_rebound(vm, &raw, "close", vec![]));
                if let PyObject::Instance { dict, .. } = &mut *args[0].borrow_mut() {
                    dict.insert_str("_closed", py_bool(true));
                }
            }
            Ok(py_none())
        }));
        td.insert("readable".into(), bf("readable", |_a| Ok(py_bool(true))));
        td.insert("writable".into(), bf("writable", |_a| Ok(py_bool(true))));
        td.insert("seekable".into(), bf("seekable", |_a| Ok(py_bool(true))));
        td.insert("__enter__".into(), bf("__enter__", |args: &[PyObjectRef]| Ok(args[0].clone())));
        td.insert("__exit__".into(), bf("__exit__", |_a| Ok(py_bool(false))));
        PyObjectRef::new(PyObject::Type {
            name: $name.to_string(),
            dict: Box::new(str_map_to_typedict(td)),
            bases: vec![$buf.clone()],
            mro: vec![$buf.clone()],
        })
    }};
}
