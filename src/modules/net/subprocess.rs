use crate::object::*;
use std::collections::HashMap;

thread_local! {
    static CALLED_PROCESS_ERROR_TYPE: std::cell::RefCell<Option<PyObjectRef>> = std::cell::RefCell::new(None);
}

/// `subprocess.CalledProcessError` — was missing entirely, so
/// `check_output`'s own non-zero-exit-status path raised a generic
/// `RuntimeError` instead of the specific, real exception real code
/// commonly catches by name (`except subprocess.CalledProcessError as e:
/// ... e.returncode`). A real `Exception`-derived `Type` (not just a
/// plain marker) exposing `.returncode`/`.cmd`/`.output`/`.stdout`/
/// `.stderr` as real instance attributes, matching real CPython's shape.
fn get_called_process_error_type() -> PyObjectRef {
    let existing = CALLED_PROCESS_ERROR_TYPE.with(|c| c.borrow().clone());
    if let Some(t) = existing {
        return t;
    }
    let mut type_dict: HashMap<String, PyObjectRef> = HashMap::new();
    type_dict.insert_str(
        "__str__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__str__".to_string(),
            func: |args| {
                if let PyObject::Instance { dict, .. } = &*args[0].borrow() {
                    let rc = dict
                        .get_str("returncode")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(-1);
                    let cmd = dict.get_str("cmd").map(|v| v.str()).unwrap_or_default();
                    Ok(py_str(&format!(
                        "Command '{}' returned non-zero exit status {}.",
                        cmd, rc
                    )))
                } else {
                    Ok(py_str(""))
                }
            },
        }),
    );
    // No native `Exception` PyObject::Type exists to list as a real base
    // here (builtin exceptions are represented as `BuiltinFunction`
    // markers elsewhere, not `Type`s) — `except CalledProcessError:`
    // matches by exact class identity via the normal Instance/Type MRO
    // walk regardless, so this is enough for the common case, just not
    // also catchable via a bare `except Exception:`.
    let typ = PyObjectRef::new(PyObject::Type {
        name: "CalledProcessError".to_string(),
        dict: Box::new(str_map_to_typedict(type_dict)),
        bases: vec![],
        mro: vec![],
    });
    // A class's own `mro` must include ITSELF (real Python: `C.__mro__[0]
    // is C`) — `except CalledProcessError as e:`'s matching walks
    // `instance.typ`'s `mro` looking for the `except` clause's referenced
    // class, so an empty `mro` here made an exact-class match fail
    // entirely (confirmed via repro: `check_output` raising this and being
    // caught by its own exact type name still fell through uncaught).
    if let PyObject::Type { mro, .. } = &mut *typ.borrow_mut() {
        *mro = vec![typ.clone()];
    }
    CALLED_PROCESS_ERROR_TYPE.with(|c| {
        *c.borrow_mut() = Some(typ.clone());
    });
    typ
}

fn make_called_process_error(
    returncode: i64,
    cmd: &str,
    output: Vec<u8>,
    stderr: Vec<u8>,
) -> PyObjectRef {
    let mut dict = crate::object::AttrMap::new();
    dict.insert_str("returncode", py_int(returncode));
    dict.insert_str("cmd", py_str(cmd));
    dict.insert_str("output", PyObjectRef::imm(PyObject::Bytes(output.clone())));
    dict.insert_str("stdout", PyObjectRef::imm(PyObject::Bytes(output)));
    dict.insert_str("stderr", PyObjectRef::imm(PyObject::Bytes(stderr)));
    PyObjectRef::new(PyObject::Instance {
        typ: get_called_process_error_type(),
        dict,
    })
}

thread_local! {
    static TIMEOUT_EXPIRED_TYPE: std::cell::RefCell<Option<PyObjectRef>> = std::cell::RefCell::new(None);
}

/// `subprocess.TimeoutExpired` — was missing entirely, and `run`/
/// `check_call`/`check_output` never enforced their `timeout=` kwarg at
/// all (they always ran the child to completion via a single blocking
/// `Command::output()`), so any real test relying on it to bound a
/// runaway/long-sleeping child (`tests/cpython/test_subprocess.py`'s
/// `test_call_timeout`/`test_timeout_exception`/`test_check_output_
/// timeout`) either ran to full completion (slow but not wrong) or, for
/// a genuinely infinite child (`"while True: pass"`), hung the whole test
/// process forever. Same real `Type`/`Instance` shape as
/// `CalledProcessError`/`CompletedProcess` above.
fn get_timeout_expired_type() -> PyObjectRef {
    let existing = TIMEOUT_EXPIRED_TYPE.with(|c| c.borrow().clone());
    if let Some(t) = existing {
        return t;
    }
    let mut type_dict: HashMap<String, PyObjectRef> = HashMap::new();
    type_dict.insert_str(
        "__str__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__str__".to_string(),
            func: |args| {
                if let PyObject::Instance { dict, .. } = &*args[0].borrow() {
                    let cmd = dict.get_str("cmd").map(|v| v.str()).unwrap_or_default();
                    // Match CPython's `str(timeout)` formatting: a caller
                    // passing a plain `int` (e.g. `timeout=-1`, the exact
                    // form `test_timeout_exception` asserts on: `"-1
                    // seconds"`, not `"-1.0 seconds"`) should not grow a
                    // spurious `.0` just because this is stored as an f64
                    // internally.
                    let timeout = dict
                        .get_str("timeout")
                        .and_then(|v| v.as_f64())
                        .map(|f| {
                            if f.fract() == 0.0 && f.is_finite() {
                                format!("{}", f as i64)
                            } else {
                                format!("{}", f)
                            }
                        })
                        .unwrap_or_else(|| "None".to_string());
                    Ok(py_str(&format!(
                        "Command '{}' timed out after {} seconds",
                        cmd, timeout
                    )))
                } else {
                    Ok(py_str(""))
                }
            },
        }),
    );
    let typ = PyObjectRef::new(PyObject::Type {
        name: "TimeoutExpired".to_string(),
        dict: Box::new(str_map_to_typedict(type_dict)),
        bases: vec![],
        mro: vec![],
    });
    if let PyObject::Type { mro, .. } = &mut *typ.borrow_mut() {
        *mro = vec![typ.clone()];
    }
    TIMEOUT_EXPIRED_TYPE.with(|c| {
        *c.borrow_mut() = Some(typ.clone());
    });
    typ
}

fn make_timeout_expired(cmd: &str, timeout: f64, output: Vec<u8>, stderr: Vec<u8>) -> PyObjectRef {
    let mut dict = crate::object::AttrMap::new();
    dict.insert_str("cmd", py_str(cmd));
    dict.insert_str("timeout", py_float(timeout));
    dict.insert_str("output", PyObjectRef::imm(PyObject::Bytes(output.clone())));
    dict.insert_str("stdout", PyObjectRef::imm(PyObject::Bytes(output)));
    dict.insert_str("stderr", PyObjectRef::imm(PyObject::Bytes(stderr)));
    PyObjectRef::new(PyObject::Instance {
        typ: get_timeout_expired_type(),
        dict,
    })
}

/// Spawn `cmd`, then poll (`try_wait`, sleeping briefly between checks)
/// until it exits or `timeout_secs` elapses. On timeout, kills the child,
/// reaps it, and returns `Err` with a `TimeoutExpired` instance ready to
/// raise. On normal exit, returns the collected `Output` (stdout/stderr
/// read to completion via `wait_with_output`, which itself drains both
/// pipes concurrently via internal threads — safe here because, unlike
/// `Popen`, this path never hands the pipes back out to Python code, so
/// there's no risk of `Popen.communicate()`'s "extra live `Rc` clone kept
/// stdin's fd open" class of bug).
fn spawn_with_timeout(
    mut command: std::process::Command,
    cmd_str: &str,
    timeout_secs: Option<f64>,
) -> Result<std::process::Output, PyError> {
    match timeout_secs {
        None => command.output().map_err(|e| PyError::os_error_from_io(&e)),
        Some(t) => {
            command.stdout(std::process::Stdio::piped());
            command.stderr(std::process::Stdio::piped());
            let mut child = command.spawn().map_err(|e| PyError::os_error_from_io(&e))?;
            let deadline = std::time::Instant::now()
                + std::time::Duration::from_secs_f64(t.max(0.0));
            loop {
                match child.try_wait() {
                    Ok(Some(_status)) => {
                        let out = child.wait_with_output();
                        return out.map_err(|e| PyError::os_error_from_io(&e));
                    }
                    Ok(None) => {
                        if std::time::Instant::now() >= deadline {
                            let _ = child.kill();
                            let _ = child.wait();
                            return Err(PyError::Exception(
                                "TimeoutExpired".to_string(),
                                make_timeout_expired(cmd_str, t, Vec::new(), Vec::new()),
                            ));
                        }
                        std::thread::sleep(std::time::Duration::from_millis(5));
                    }
                    Err(e) => return Err(PyError::os_error_from_io(&e)),
                }
            }
        }
    }
}

/// Extract a `timeout=` kwarg (float seconds) from the trailing kwargs
/// pack, if present and not `None`.
fn kwarg_timeout(args: &[PyObjectRef]) -> Option<f64> {
    match args.last() {
        Some(last) if matches!(&*last.borrow(), PyObject::Dict(_)) => {
            if let PyObject::Dict(d) = &*last.borrow() {
                d.get(&py_str("timeout")).ok().flatten().and_then(|v| {
                    if matches!(&*v.borrow(), PyObject::None) {
                        None
                    } else {
                        v.as_f64()
                    }
                })
            } else {
                None
            }
        }
        _ => None,
    }
}

thread_local! {
    static COMPLETED_PROCESS_TYPE: std::cell::RefCell<Option<PyObjectRef>> = std::cell::RefCell::new(None);
}

/// `subprocess.CompletedProcess` — `subprocess.run(...)` previously
/// returned a bare `dict` (`{"returncode": ..., "stdout": ..., "stderr":
/// ...}`) instead of a real object with `.returncode`/`.stdout`/`.stderr`
/// ATTRIBUTE access, which is how every real caller uses it
/// (`subprocess.run(...).returncode`) — `result["returncode"]` happened to
/// also work by accident (dict subscript), but `.returncode` raised
/// `AttributeError: 'dict' object has no attribute 'returncode'`. Same
/// shape as `CalledProcessError` just above: a real `Type`/`Instance` pair,
/// not a plain marker.
fn get_completed_process_type() -> PyObjectRef {
    let existing = COMPLETED_PROCESS_TYPE.with(|c| c.borrow().clone());
    if let Some(t) = existing {
        return t;
    }
    let mut type_dict: HashMap<String, PyObjectRef> = HashMap::new();
    type_dict.insert_str(
        "__repr__",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "__repr__".to_string(),
            func: |args| {
                if let PyObject::Instance { dict, .. } = &*args[0].borrow() {
                    let rc = dict
                        .get_str("returncode")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0);
                    let args_repr = dict.get_str("args").map(|v| v.repr()).unwrap_or_default();
                    Ok(py_str(&format!(
                        "CompletedProcess(args={}, returncode={})",
                        args_repr, rc
                    )))
                } else {
                    Ok(py_str("CompletedProcess(...)"))
                }
            },
        }),
    );
    type_dict.insert_str(
        "check_returncode",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "check_returncode".to_string(),
            func: |args| {
                if let PyObject::Instance { dict, .. } = &*args[0].borrow() {
                    let rc = dict
                        .get_str("returncode")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0);
                    if rc != 0 {
                        let cmd = dict.get_str("args").map(|v| v.str()).unwrap_or_default();
                        let stdout = dict
                            .get_str("stdout")
                            .map(|v| v.str().into_bytes())
                            .unwrap_or_default();
                        let stderr = dict
                            .get_str("stderr")
                            .map(|v| v.str().into_bytes())
                            .unwrap_or_default();
                        let err = make_called_process_error(rc, &cmd, stdout, stderr);
                        return Err(PyError::Exception("CalledProcessError".to_string(), err));
                    }
                }
                Ok(py_none())
            },
        }),
    );
    let typ = PyObjectRef::new(PyObject::Type {
        name: "CompletedProcess".to_string(),
        dict: Box::new(str_map_to_typedict(type_dict)),
        bases: vec![],
        mro: vec![],
    });
    if let PyObject::Type { mro, .. } = &mut *typ.borrow_mut() {
        *mro = vec![typ.clone()];
    }
    COMPLETED_PROCESS_TYPE.with(|c| {
        *c.borrow_mut() = Some(typ.clone());
    });
    typ
}

fn make_completed_process(
    cmd_args: PyObjectRef,
    returncode: i64,
    stdout: PyObjectRef,
    stderr: PyObjectRef,
) -> PyObjectRef {
    let mut dict = crate::object::AttrMap::new();
    dict.insert_str("args", cmd_args);
    dict.insert_str("returncode", py_int(returncode));
    dict.insert_str("stdout", stdout);
    dict.insert_str("stderr", stderr);
    PyObjectRef::new(PyObject::Instance {
        typ: get_completed_process_type(),
        dict,
    })
}

pub fn create_subprocess_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! sub_func {
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
    d.insert_str("CalledProcessError", get_called_process_error_type());
    d.insert_str("TimeoutExpired", get_timeout_expired_type());

    // Real `subprocess.run`/`check_call`/`check_output` all have `shell` as
    // a KEYWORD-ONLY parameter (`def run(args, *, shell=False, ...)`) — this
    // used to treat `args[1]` as if it were a POSITIONAL `shell` flag
    // instead. Since keyword arguments arrive packed into a trailing `Dict`
    // (this project's own established calling convention), ANY call using
    // ANY keyword argument at all — `capture_output=True`, `text=True`,
    // `check=True`, `timeout=...`, all extremely common — put that non-
    // empty kwargs dict in `args[1]`, and `args[1].truthy()` on a non-empty
    // dict is always `True`: every such call silently ran in `shell=True`
    // mode instead, feeding the WHOLE debug-stringified command list
    // (`"['/path/to/python', '-c', ...]"`, brackets and all) to `/bin/sh
    // -c` as one command — confirmed via direct repro (`subprocess.run(cmd,
    // capture_output=True, text=True)` failed with `/bin/sh: ... [/path...,:
    // No such file or directory`, the literal Rust-debug-formatted list).
    // Looks up `shell` specifically BY NAME in the trailing kwargs dict
    // instead.
    fn kwarg_bool(args: &[PyObjectRef], name: &str) -> bool {
        match args.last() {
            Some(last) if matches!(&*last.borrow(), PyObject::Dict(_)) => {
                if let PyObject::Dict(d) = &*last.borrow() {
                    d.get(&py_str(name))
                        .ok()
                        .flatten()
                        .map(|v| v.truthy())
                        .unwrap_or(false)
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    // Extract a keyword dict argument (e.g. `env=`) from the trailing
    // kwargs pack.
    fn kwarg_dict(
        args: &[PyObjectRef],
        name: &str,
    ) -> Option<std::collections::HashMap<String, String>> {
        match args.last() {
            Some(last) if matches!(&*last.borrow(), PyObject::Dict(_)) => {
                if let PyObject::Dict(d) = &*last.borrow() {
                    if let Some(v) = d.get(&py_str(name)).ok().flatten() {
                        if let PyObject::Dict(ed) = &*v.borrow() {
                            let mut m = std::collections::HashMap::new();
                            for (k, val) in ed.items() {
                                m.insert(k.str(), val.str());
                            }
                            return Some(m);
                        }
                    }
                }
                None
            }
            _ => None,
        }
    }

    // Apply the `env=` kwargs (replacing the inherited environment
    // entirely, matching CPython) to a command.
    fn apply_env(cmd: &mut std::process::Command, args: &[PyObjectRef]) {
        if let Some(env_map) = kwarg_dict(args, "env") {
            cmd.env_clear();
            for (k, v) in env_map {
                cmd.env(k, v);
            }
        }
    }

    sub_func!("run", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("run() missing required argument"));
        }
        let shell = kwarg_bool(args, "shell");
        let cmd_str = args[0].str();
        if cmd_str.is_empty() {
            return Err(PyError::ValueError("empty command".to_string()));
        }
        let timeout = kwarg_timeout(args);
        let output = if shell {
            let mut cmd = std::process::Command::new("/bin/sh");
            cmd.arg("-c").arg(&cmd_str);
            apply_env(&mut cmd, args);
            spawn_with_timeout(cmd, &cmd_str, timeout)?
        } else {
            let cmd_args: Vec<String> = if let PyObject::List(items) = &*args[0].borrow() {
                items.iter().map(|a| a.str()).collect()
            } else {
                vec![cmd_str.clone()]
            };
            if cmd_args.is_empty() {
                return Err(PyError::ValueError("empty command".to_string()));
            }
            let mut cmd = std::process::Command::new(&cmd_args[0]);
            cmd.args(&cmd_args[1..]);
            apply_env(&mut cmd, args);
            spawn_with_timeout(cmd, &cmd_str, timeout)?
        };
        let returncode = output.status.code().unwrap_or(-1) as i64;
        let stdout_str = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr_str = String::from_utf8_lossy(&output.stderr).to_string();
        Ok(make_completed_process(
            args[0].clone(),
            returncode,
            py_str(&stdout_str),
            py_str(&stderr_str),
        ))
    });

    // `subprocess.call` — was missing entirely (`AttributeError`), even
    // though it's one of the most common convenience wrappers real code
    // uses (`tests/cpython/test_subprocess.py`'s `test_call_timeout`
    // among others). Same shape as `check_call` just below, minus the
    // non-zero-exit-status check — just returns the returncode.
    sub_func!("call", |args| {
        if args.is_empty() {
            return Err(PyError::type_error("call() missing required argument"));
        }
        let cmd_str = args[0].str();
        let shell = kwarg_bool(args, "shell");
        let timeout = kwarg_timeout(args);
        let output = if shell {
            let mut cmd = std::process::Command::new("sh");
            cmd.arg("-c").arg(&cmd_str);
            apply_env(&mut cmd, args);
            spawn_with_timeout(cmd, &cmd_str, timeout)?
        } else {
            let cmd_args: Vec<String> = if let PyObject::List(items) = &*args[0].borrow() {
                items.iter().map(|a| a.str()).collect()
            } else {
                cmd_str.split_whitespace().map(|s| s.to_string()).collect()
            };
            if cmd_args.is_empty() {
                return Err(PyError::type_error("call() requires a non-empty command"));
            }
            let mut cmd = std::process::Command::new(&cmd_args[0]);
            cmd.args(&cmd_args[1..]);
            apply_env(&mut cmd, args);
            spawn_with_timeout(cmd, &cmd_str, timeout)?
        };
        Ok(py_int(output.status.code().unwrap_or(-1) as i64))
    });

    sub_func!("check_call", |args| {
        // Run a command and check return code
        if args.is_empty() {
            return Err(PyError::type_error(
                "check_call() missing required argument",
            ));
        }
        let cmd_str = args[0].str();
        let shell = kwarg_bool(args, "shell");
        let timeout = kwarg_timeout(args);
        let output = if shell {
            let mut cmd = std::process::Command::new("sh");
            cmd.arg("-c").arg(&cmd_str);
            apply_env(&mut cmd, args);
            spawn_with_timeout(cmd, &cmd_str, timeout)?
        } else {
            // Was: `cmd_str.split_whitespace()` — the LIST form (the common
            // case, e.g. `[sys.executable, '-E', '-c', code]`) was stringified
            // via `.str()` first (producing the Rust-debug-ish
            // `"['/path/to/exe', '-E', ...]"`) and THEN split on whitespace,
            // shredding the executable path into garbage tokens like
            // `"['/path/to/exe',"` — every list-form call failed with a
            // spurious `OSError: No such file or directory`. Mirrors the
            // already-correct `run`/`check_output` handling just above/below.
            let cmd_args: Vec<String> = if let PyObject::List(items) = &*args[0].borrow() {
                items.iter().map(|a| a.str()).collect()
            } else {
                cmd_str.split_whitespace().map(|s| s.to_string()).collect()
            };
            if cmd_args.is_empty() {
                return Err(PyError::type_error(
                    "check_call() requires a non-empty command",
                ));
            }
            let mut cmd = std::process::Command::new(&cmd_args[0]);
            cmd.args(&cmd_args[1..]);
            apply_env(&mut cmd, args);
            spawn_with_timeout(cmd, &cmd_str, timeout)?
        };
        let returncode = output.status.code().unwrap_or(-1);
        if returncode != 0 {
            return Err(PyError::runtime_error(format!(
                "Command returned non-zero exit status {}",
                returncode
            )));
        }
        Ok(py_none())
    });

    sub_func!("check_output", |args| {
        if args.is_empty() {
            return Err(PyError::type_error(
                "check_output() missing required argument",
            ));
        }
        let shell = kwarg_bool(args, "shell");
        let cmd_str = args[0].str();
        if cmd_str.is_empty() {
            return Err(PyError::ValueError("empty command".to_string()));
        }
        let timeout = kwarg_timeout(args);
        let output = if shell {
            let mut cmd = std::process::Command::new("/bin/sh");
            cmd.arg("-c").arg(&cmd_str);
            apply_env(&mut cmd, args);
            spawn_with_timeout(cmd, &cmd_str, timeout)?
        } else {
            let cmd_args: Vec<String> = if let PyObject::List(items) = &*args[0].borrow() {
                items.iter().map(|a| a.str()).collect()
            } else {
                vec![cmd_str.clone()]
            };
            if cmd_args.is_empty() {
                return Err(PyError::ValueError("empty command".to_string()));
            }
            let mut cmd = std::process::Command::new(&cmd_args[0]);
            cmd.args(&cmd_args[1..]);
            apply_env(&mut cmd, args);
            spawn_with_timeout(cmd, &cmd_str, timeout)?
        };
        if !output.status.success() {
            let rc = output.status.code().unwrap_or(-1) as i64;
            let instance =
                make_called_process_error(rc, &args[0].str(), output.stdout, output.stderr);
            return Err(PyError::Exception(
                "CalledProcessError".to_string(),
                instance,
            ));
        }
        // Real CPython decodes stdout to `str` when `text=True` /
        // `universal_newlines=True` / a truthy `encoding=`/`errors=` is
        // given (any one of these enables "text mode" — check all four,
        // matching `Popen.__init__`'s own `text is encoding is errors is
        // None` no-text-mode test). Returning raw bytes unconditionally
        // broke any caller expecting a real string back (real trigger:
        // `Lib/platform.py`'s `_Processor.from_subprocess()`, which calls
        // `subprocess.check_output([...], text=True, encoding="utf8")`
        // and then does `.strip()` on the result — `TypeError: sequence
        // item ...: expected str instance, bytes found` once that `str`
        // got embedded in another string-join downstream).
        let wants_text = kwarg_bool(&args, "text")
            || kwarg_bool(&args, "universal_newlines")
            || matches!(args.last(), Some(last) if matches!(&*last.borrow(), PyObject::Dict(_)))
                && {
                    if let PyObject::Dict(d) = &*args.last().unwrap().borrow() {
                        ["encoding", "errors"].iter().any(|k| {
                            d.get(&py_str(k))
                                .ok()
                                .flatten()
                                .map(|v| !matches!(&*v.borrow(), PyObject::None))
                                .unwrap_or(false)
                        })
                    } else {
                        false
                    }
                };
        if wants_text {
            let text = String::from_utf8_lossy(&output.stdout).into_owned();
            Ok(py_str(&text))
        } else {
            Ok(PyObjectRef::imm(PyObject::Bytes(output.stdout)))
        }
    });

    // Real CPython's internal `subprocess._cleanup()` reaps any finished
    // background children it's still tracking. `Lib/test/support/script_
    // helper.py`'s `run_python_until_end` calls it unconditionally in a
    // `finally` block after every spawned test subprocess — it was missing
    // entirely (`AttributeError`), so EVERY test using `assert_python_ok`/
    // `assert_python_failure` failed there even after the process itself
    // ran and exited correctly. This project's `Popen`/`run`/`check_call`
    // all block synchronously with no background-process registry, so
    // there is nothing to reap — a no-op is behavior-equivalent.
    sub_func!("_cleanup", |_args| Ok(py_none()));

    // `subprocess._active` — the list of `Popen` instances not yet
    // `.wait()`-ed, real CPython's own background-reap registry. Missing
    // entirely broke `tests/cpython/test_subprocess.py`'s
    // `BaseTestCase.tearDown` (run after EVERY test in the file) with
    // `AttributeError: module 'subprocess' has no attribute '_active'` —
    // it iterates `subprocess._active` unconditionally. Same rationale as
    // `_cleanup` just above: this module's `Popen`/`run`/`check_call` all
    // block synchronously with no background registry, so an always-empty
    // list is behavior-equivalent (nothing is ever "still active" to
    // report) and satisfies `assertFalse(subprocess._active)`.
    d.insert_str("_active", PyObjectRef::new(PyObject::List(vec![])));

    // Constants
    d.insert_str("PIPE", py_int(-1));
    d.insert_str("STDOUT", py_int(-2));
    d.insert_str("DEVNULL", py_int(-3));

    // `subprocess.Popen` — was missing ENTIRELY (only the higher-level
    // `run`/`check_output` convenience wrappers existed, both of which
    // block synchronously via `Command::output()`). Real CPython's own
    // `run`/`check_output` are themselves implemented ON TOP OF `Popen` —
    // it's the foundational class real code reaches for whenever it needs
    // non-blocking spawn, streaming I/O, or manual process lifecycle
    // control (`.poll()`, `.wait()`, `.communicate()`, `.terminate()`).
    // Spawns eagerly (matching real `Popen` semantics — construction
    // itself starts the child process) via `Command::spawn()` (non-
    // blocking, unlike `.output()`). `stdin`/`stdout`/`stderr` accept
    // `subprocess.PIPE`/`DEVNULL`/`STDOUT` (int sentinels) or default to
    // inherited (matching real Python's own `None` default).
    sub_func!("Popen", |args| {
        if args.is_empty() {
            return Err(PyError::type_error(
                "Popen() missing required argument 'args'",
            ));
        }
        let kwargs = args.last().and_then(|a| {
            if let PyObject::Dict(d) = &*a.borrow() {
                Some(d.clone())
            } else {
                None
            }
        });
        let get_kw = |name: &str| {
            kwargs
                .as_ref()
                .and_then(|d| d.get(&py_str(name)).ok().flatten())
        };
        let shell = get_kw("shell").map(|v| v.truthy()).unwrap_or(false);
        let cmd_arg = &args[0];

        let mut command = if shell {
            let cmd_str = cmd_arg.str();
            let mut c = std::process::Command::new("/bin/sh");
            c.arg("-c").arg(&cmd_str);
            c
        } else {
            let cmd_args: Vec<String> = match &*cmd_arg.borrow() {
                PyObject::List(items) | PyObject::Tuple(items) => {
                    items.iter().map(|a| a.str()).collect()
                }
                _ => vec![cmd_arg.str()],
            };
            if cmd_args.is_empty() {
                return Err(PyError::value_error("empty command"));
            }
            let mut c = std::process::Command::new(&cmd_args[0]);
            c.args(&cmd_args[1..]);
            c
        };
        if let Some(cwd) = get_kw("cwd") {
            if !matches!(&*cwd.borrow(), PyObject::None) {
                command.current_dir(crate::object::path_arg_to_string(&cwd));
            }
        }
        if let Some(env_kw) = get_kw("env") {
            // An explicit env REPLACES the inherited environment entirely
            // (CPython semantics — the test suite relies on it for
            // PYTHONHASHSEED).
            if let PyObject::Dict(ed) = &*env_kw.borrow() {
                command.env_clear();
                for (k, v) in ed.items() {
                    command.env(k.str(), v.str());
                }
            }
        }
        // Sentinel ints (matching this module's own PIPE/STDOUT/DEVNULL
        // constants just above) map to the corresponding `Stdio` mode;
        // a non-negative int is a real, already-open file descriptor the
        // caller wants the child to inherit AS that specific stream (real
        // CPython: `Popen(..., stdout=some_fd)`) — this used to fall
        // through to `Stdio::inherit()` same as the `None` default,
        // silently handing the child THIS PROCESS's own real stdout/
        // stderr/stdin instead of the fd the caller actually asked for.
        // Real trigger: `test_pass_fds_redirected` passes a pipe's write
        // end as `stdout=`, then does a blocking `os.read()` on the read
        // end expecting the child's output there — with the fd ignored,
        // the child wrote to the test HARNESS's own terminal instead, and
        // the `os.read()` waiting for data that would now never arrive
        // hung forever. Duplicate the fd (matching CPython's own
        // dup-into-the-child semantics: the ORIGINAL fd must stay open
        // and valid in the parent, e.g. for the caller's own cleanup
        // logic) via a raw `File` wrapper's `try_clone()`, which
        // duplicates without taking ownership of/closing the original.
        // Anything else (including the real default, `None`) inherits
        // the parent's stream, matching real Python's own default.
        let stdio_for = |v: &PyObjectRef| -> std::process::Stdio {
            match v.as_i64() {
                Some(-1) => std::process::Stdio::piped(),
                Some(-3) => std::process::Stdio::null(),
                Some(fd) if fd >= 0 => {
                    use std::os::unix::io::FromRawFd;
                    let borrowed =
                        std::mem::ManuallyDrop::new(unsafe {
                            std::fs::File::from_raw_fd(fd as i32)
                        });
                    match borrowed.try_clone() {
                        Ok(dup) => std::process::Stdio::from(dup),
                        Err(_) => std::process::Stdio::inherit(),
                    }
                }
                _ => std::process::Stdio::inherit(),
            }
        };
        // `stderr=STDOUT` (-2): the child's stdout and stderr must flow into
        // the SAME pipe (CPython merges them; test_cmd_line_script's
        // interactive_python drains the combined stream looking for the
        // REPL prompt, which is written to stderr). Rust's `Command` has no
        // "stderr = stdout" `Stdio`, so build a socketpair, hand BOTH the
        // child's stdout and stderr the same write end, and expose the read
        // end as both `.stdout` and `.stderr`.
        let stderr_is_stdout = get_kw("stderr")
            .map(|v| v.as_i64() == Some(-2))
            .unwrap_or(false);
        let mut merged_read_end: Option<std::rc::Rc<std::cell::RefCell<std::fs::File>>> = None;
        if stderr_is_stdout {
            use std::os::fd::OwnedFd;
            use std::os::unix::io::IntoRawFd;
            if let Ok((read_sock, write_sock)) = std::os::unix::net::UnixStream::pair() {
                let write2 = write_sock.try_clone();
                let read_file = unsafe { std::fs::File::from_raw_fd(read_sock.into_raw_fd()) };
                merged_read_end = Some(std::rc::Rc::new(std::cell::RefCell::new(read_file)));
                command.stdout(std::process::Stdio::from(unsafe {
                    OwnedFd::from_raw_fd(write_sock.into_raw_fd())
                }));
                if let Ok(w2) = write2 {
                    command.stderr(std::process::Stdio::from(unsafe {
                        OwnedFd::from_raw_fd(w2.into_raw_fd())
                    }));
                } else {
                    command.stderr(std::process::Stdio::null());
                }
            }
        }
        if let Some(v) = get_kw("stdin") {
            command.stdin(stdio_for(&v));
        }
        if !stderr_is_stdout {
            if let Some(v) = get_kw("stdout") {
                command.stdout(stdio_for(&v));
            }
            if let Some(v) = get_kw("stderr") {
                match v.as_i64() {
                    Some(-2) => {
                        command.stderr(std::process::Stdio::piped());
                    }
                    _ => {
                        command.stderr(stdio_for(&v));
                    }
                }
            }
        }

        let mut child = command.spawn().map_err(|e| PyError::os_error_from_io(&e))?;
        let pid = child.id() as i64;
        // Take the piped ends into the Process so `.stdin`/`.stdout`/
        // `.stderr` expose real, readable/writable file objects (not dummy
        // /dev/null — the interactive REPL tests write a statement then
        // read the prompt back, which needs the actual pipe).
        use std::os::unix::io::{FromRawFd, IntoRawFd};
        let wrap_pipe = |p: Option<std::process::ChildStdout>| -> Option<std::rc::Rc<std::cell::RefCell<std::fs::File>>> {
            p.map(|s| std::rc::Rc::new(std::cell::RefCell::new(unsafe { std::fs::File::from_raw_fd(s.into_raw_fd()) })))
        };
        let wrap_pipe_in = |p: Option<std::process::ChildStdin>| -> Option<std::rc::Rc<std::cell::RefCell<std::fs::File>>> {
            p.map(|s| std::rc::Rc::new(std::cell::RefCell::new(unsafe { std::fs::File::from_raw_fd(s.into_raw_fd()) })))
        };
        let wrap_pipe_err = |p: Option<std::process::ChildStderr>| -> Option<std::rc::Rc<std::cell::RefCell<std::fs::File>>> {
            p.map(|s| std::rc::Rc::new(std::cell::RefCell::new(unsafe { std::fs::File::from_raw_fd(s.into_raw_fd()) })))
        };
        let (stdout_pipe, stderr_pipe) = if let Some(m) = merged_read_end {
            (Some(m.clone()), Some(m))
        } else {
            (
                wrap_pipe(child.stdout.take()),
                wrap_pipe_err(child.stderr.take()),
            )
        };
        let stdin_pipe = wrap_pipe_in(child.stdin.take());
        Ok(PyObjectRef::new(PyObject::Process {
            child: std::rc::Rc::new(std::cell::RefCell::new(Some(child))),
            returncode: std::rc::Rc::new(std::cell::RefCell::new(None)),
            pid,
            stdin_pipe,
            stdout_pipe,
            stderr_pipe,
        }))
    });

    d
}

