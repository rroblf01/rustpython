use crate::object::*;
use std::collections::HashMap;
use std::sync::atomic::AtomicI64;
use num_traits::{Signed, ToPrimitive};
use std::rc::Rc;


/// Helper: resolve a module name with relative import support
fn resolve_name(name: &str, package: Option<&str>) -> Result<String, PyError> {
    if !name.starts_with('.') {
        return Ok(name.to_string());
    }
    let pkg = match package {
        Some(p) => p.to_string(),
        None => {
            return Err(PyError::type_error(
                "import_module() requires 'package' argument for relative import",
            ))
        }
    };
    let level = name.chars().take_while(|&c| c == '.').count();
    let rel_part = &name[level..];
    let pkg_parts: Vec<&str> = pkg.split('.').collect();
    if level > pkg_parts.len() {
        return Err(PyError::ImportError(
            "attempted relative import beyond top-level package".to_string(),
        ));
    }
    let base = &pkg_parts[..pkg_parts.len() - level];
    if base.is_empty() {
        Ok(rel_part.to_string())
    } else if rel_part.is_empty() {
        Ok(base.join("."))
    } else {
        Ok(format!("{}.{}", base.join("."), rel_part))
    }
}

/// Helper: import a dotted module chain, ensuring parents are loaded first
fn import_dotted(vm: &mut crate::vm::VirtualMachine, name: &str) -> PyResult<PyObjectRef> {
    // If it's already loaded, return it
    if let Some(module) = vm.modules.get(name) {
        return Ok(module.clone());
    }
    // For dotted names, load the chain step by step
    if name.contains('.') {
        let parts: Vec<&str> = name.split('.').collect();
        let mut current = String::new();
        for part in &parts {
            if current.is_empty() {
                current = part.to_string();
            } else {
                current = format!("{}.{}", current, part);
            }
            if !vm.modules.contains_key(&current) {
                let module = vm.import_module_from_file(&current)?;
                vm.modules.insert(current.clone(), module.clone());
                // Also sync to sys.modules
                if let Some(sys_mod) = vm.modules.get("sys") {
                    if let PyObject::Module { dict, .. } = &*sys_mod.borrow() {
                        if let Some(mod_dict) = dict.get_str("modules") {
                            mod_dict.borrow_mut().set_attribute(&current, module).ok();
                        }
                    }
                }
            }
        }
        if let Some(module) = vm.modules.get(name) {
            return Ok(module.clone());
        }
        return Err(PyError::module_not_found_error(format!(
            "No module named '{}'",
            name
        )));
    }
    // Simple name
    let module = vm.import_module_from_file(name)?;
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

/// `importlib.import_module(name, package=None)`. A genuine, named,
/// top-level function (not an inline closure like this module's other
/// builtins) specifically so `vm.rs`'s `call_function` can recognize it
/// by function-pointer identity and special-case it — matching
/// `type.__new__`/`getattr` above. `with_vm_mut` below is only a
/// fallback for the (currently believed unreachable, since every real
/// call goes through a normal `CALL`/`CALL_KW` opcode) case of being
/// invoked some other way; the aliasing hazard it otherwise risks
/// (see `with_vm_mut`'s own doc comment) is why the special case exists.
pub(crate) fn import_module_builtin(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error(
            "import_module() missing required argument 'name'",
        ));
    }
    let name = args[0].str();
    let package: Option<String> = if args.len() >= 2 {
        let pkg = args[1].str();
        if pkg.is_empty() {
            None
        } else {
            Some(pkg)
        }
    } else {
        None
    };

    // Resolve relative imports
    let resolved = resolve_name(&name, package.as_deref())?;

    // Use with_vm_mut for VM-dependent part
    with_vm_mut(|vm| -> PyResult<PyObjectRef> {
        if let Some(module) = vm.modules.get(&resolved) {
            return Ok(module.clone());
        }
        import_dotted(vm, &resolved)
    })?
}

/// Shared by both `call_function`'s special case (the normal path) and
/// the plain-`BuiltinFunction` fallback: resolves relative imports and
/// returns the already-loaded module or imports it fresh via `vm`.
pub(crate) fn import_module_with_vm(
    vm: &mut crate::vm::VirtualMachine,
    name: &str,
    package: Option<&str>,
) -> PyResult<PyObjectRef> {
    let resolved = resolve_name(name, package)?;
    if let Some(module) = vm.modules.get(&resolved) {
        return Ok(module.clone());
    }
    import_dotted(vm, &resolved)
}

/// Native importlib stub module providing import_module().
pub fn create_importlib_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    d.insert_str(
        "import_module",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "import_module".to_string(),
            func: import_module_builtin,
        }),
    );
    // __version__ — indicates importlib metadata
    d.insert_str("__version__", py_str("1.0.0"));
    // `importlib.invalidate_caches()` — real CPython clears internal
    // finder/loader caches so newly-created files on disk (a common test
    // pattern: write a module file, then import it) are found. This
    // interpreter's own import machinery doesn't maintain any such cache to
    // begin with (every import does a fresh filesystem lookup), so a no-op
    // is the correct, safe simplification — missing entirely before raised
    // `AttributeError`, breaking any test that merely CALLS this for
    // hygiene even when it doesn't strictly need caches invalidated (real
    // trigger: CPython's own `test_cmd_line_script.py`/`test_tokenize.py`/
    // others).
    d.insert_str(
        "invalidate_caches",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "invalidate_caches".to_string(),
            func: |_args| Ok(py_none()),
        }),
    );
    d
}

/// Native importlib.util module providing find_spec().
pub fn create_importlib_util_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();
    macro_rules! util_func {
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

    // find_spec(name, package=None) -> ModuleSpec or None
    util_func!("find_spec", find_spec_builtin);

    // cache_from_source(path, ...)/source_from_cache(path) — real CPython's
    // `__pycache__/name.cpython-VER.pyc` naming convention. Implemented as
    // plain string manipulation (not tied to this interpreter's own actual
    // bytecode-cache format) — good enough for code that just constructs/
    // parses the conventional path shape (real trigger: `py_compile.py`,
    // vendored verbatim, needs `cache_from_source` to pick a default output
    // path for `py_compile.compile()`).
    util_func!("cache_from_source", |args| {
        if args.is_empty() {
            return Err(PyError::type_error(
                "cache_from_source() missing required argument: 'path'",
            ));
        }
        let path = args[0].str();
        let (dir, base) = match path.rfind('/') {
            Some(i) => (path[..i].to_string(), path[i + 1..].to_string()),
            None => (String::new(), path.clone()),
        };
        let stem = base.strip_suffix(".py").unwrap_or(&base);
        let cache_dir = if dir.is_empty() {
            "__pycache__".to_string()
        } else {
            format!("{}/__pycache__", dir)
        };
        Ok(py_str(&format!("{}/{}.cpython-314.pyc", cache_dir, stem)))
    });
    util_func!("source_from_cache", |args| {
        if args.is_empty() {
            return Err(PyError::type_error(
                "source_from_cache() missing required argument: 'path'",
            ));
        }
        let path = args[0].str();
        if !path.ends_with(".pyc") {
            return Err(PyError::value_error("not a valid pyc path"));
        }
        let without_pycache = path.replace("/__pycache__/", "/");
        let base = without_pycache
            .rsplit('/')
            .next()
            .unwrap_or(&without_pycache);
        let dir = without_pycache[..without_pycache.len() - base.len()].to_string();
        let stem = base.split(".cpython-").next().unwrap_or(base);
        Ok(py_str(&format!("{}{}.py", dir, stem)))
    });

    d
}

/// The real body of `importlib.util.find_spec`, given genuine `&mut
/// VirtualMachine` access — called directly from `vm.rs`'s `call_function`
/// special-case (see the `is_find_spec` check there) instead of through
/// `find_spec_builtin`'s `with_vm_mut` fallback below, since this function is
/// always reached from deep inside an active VM call chain in practice
/// (Django's app-loading machinery calls it while `apps.populate()` is
/// running), and `with_vm_mut` there reborrows the *same* `VirtualMachine`
/// mutably while an outer `&mut self` is already live on the Rust call stack
/// — a real, confirmed aliasing UB (`hashbrown`'s `HashMap::contains_key`
/// segfaulting on a corrupted table, non-deterministically, since the bug is
/// UB and not always "caught"), not merely a style concern.
pub(crate) fn find_spec_with_vm(
    vm: &mut crate::vm::VirtualMachine,
    name: &str,
    package: Option<&str>,
) -> PyResult<PyObjectRef> {
    // Resolve the full module name (handle relative imports)
    let resolved_name = if let Some(pkg) = package {
        if name.starts_with('.') {
            let level = name.chars().take_while(|&c| c == '.').count();
            let rel_part = &name[level..];
            let pkg_parts: Vec<&str> = pkg.split('.').collect();
            if level > pkg_parts.len() {
                return Ok(py_none());
            }
            let base = &pkg_parts[..pkg_parts.len() - level];
            if base.is_empty() {
                rel_part.to_string()
            } else if rel_part.is_empty() {
                base.join(".")
            } else {
                format!("{}.{}", base.join("."), rel_part)
            }
        } else if !name.contains('.') {
            format!("{}.{}", pkg, name)
        } else {
            name.to_string()
        }
    } else {
        name.to_string()
    };

    if vm.modules.contains_key(&resolved_name) {
        return Ok(create_module(
            "ModuleSpec",
            HashMap::from([
                ("name".to_string(), py_str(&resolved_name)),
                ("origin".to_string(), py_str("built-in")),
            ]),
        ));
    }

    // Get sys.path manually to search for the module file
    let mut search_paths: Vec<String> = Vec::new();
    if let Some(sys_mod) = vm.modules.get("sys") {
        if let PyObject::Module { dict, .. } = &*sys_mod.borrow() {
            if let Some(path_list) = dict.get_str("path") {
                if let PyObject::List(items) = &*path_list.borrow() {
                    for item in items {
                        if let PyObject::Str(s) = &*item.borrow() {
                            search_paths.push(s.to_string());
                        }
                    }
                }
            }
        }
    }

    // For dotted names, we need to find the file for the top-level
    let top_name = if resolved_name.contains('.') {
        resolved_name.split('.').next().unwrap().to_string()
    } else {
        resolved_name.clone()
    };

    // Search the filesystem for the module
    for base in &search_paths {
        let base_trimmed = base.trim_end_matches('/');
        let py_path = format!("{}/{}.py", base_trimmed, top_name);
        if std::path::Path::new(&py_path).exists() {
            return Ok(create_module(
                "ModuleSpec",
                HashMap::from([
                    ("name".to_string(), py_str(&resolved_name)),
                    ("origin".to_string(), py_str(&py_path)),
                ]),
            ));
        }
        let init_path = format!("{}/{}/__init__.py", base_trimmed, top_name);
        if std::path::Path::new(&init_path).exists() {
            return Ok(create_module(
                "ModuleSpec",
                HashMap::from([
                    ("name".to_string(), py_str(&resolved_name)),
                    ("origin".to_string(), py_str(&init_path)),
                ]),
            ));
        }
    }

    Ok(py_none())
}

/// `find_spec`'s standalone entry point (used when it's not reached through
/// `vm.rs`'s special-cased dispatch) — falls back to `with_vm_mut`, matching
/// `import_module_builtin`'s role for `importlib.import_module`.
pub(crate) fn find_spec_builtin(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
    if args.is_empty() {
        return Err(PyError::type_error(
            "find_spec() missing required argument 'name'",
        ));
    }
    let name = args[0].str();
    let package = if args.len() >= 2 {
        let pkg = args[1].str();
        if pkg.is_empty() {
            None
        } else {
            Some(pkg)
        }
    } else {
        None
    };
    Ok(with_vm_mut(|vm| {
        find_spec_with_vm(vm, &name, package.as_deref())
    })??)
}

/// Native importlib.resources stub module.
/// Provides `files(package)` and `as_file(traversable)` stubs for certifi compatibility.
pub fn create_importlib_resources_dict() -> HashMap<String, PyObjectRef> {
    let mut d = HashMap::new();

    // Helper: read name attribute from a module
    fn mod_name(obj: &PyObjectRef) -> String {
        let b = obj.borrow();
        if let PyObject::Module { dict, .. } = &*b {
            if let Some(name) = dict.get_str("name") {
                if let PyObject::Str(s) = &*name.borrow() {
                    return s.to_string();
                }
            }
        }
        String::new()
    }

    // __enter__ for context manager: return args[0].name
    fn enter_cm(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
        if args.is_empty() {
            return Ok(py_none());
        }
        Ok(py_str(&mod_name(&args[0])))
    }

    // __exit__ for context manager: no-op
    fn exit_cm(_args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
        Ok(py_none())
    }

    // joinpath for traversable: args[0].name + args[1], returns new Traversable
    fn trav_joinpath(args: &[PyObjectRef]) -> PyResult<PyObjectRef> {
        if args.len() < 2 {
            return Ok(py_none());
        }
        let base = mod_name(&args[0]);
        let child = args[1].str();
        let joined = format!(
            "{}/{}",
            base.trim_end_matches('/'),
            child.trim_start_matches('/')
        );
        let trav = create_module(
            "_Traversable",
            HashMap::from([("name".to_string(), py_str(&joined))]),
        );
        // Add joinpath as BuiltinMethod with self_obj = trav
        if let PyObject::Module { dict, .. } = &mut *trav.borrow_mut() {
            dict.insert_str(
                "joinpath",
                PyObjectRef::new(PyObject::BuiltinMethod {
                    name: "joinpath".to_string(),
                    func: trav_joinpath,
                    self_obj: trav.clone(),
                }),
            );
        }
        Ok(trav)
    }

    // as_file(traversable) -> context manager wrapping the path
    d.insert_str(
        "as_file",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "as_file".to_string(),
            func: |args| {
                if args.is_empty() {
                    return Err(PyError::type_error(
                        "as_file() requires 1 argument (traversable)",
                    ));
                }
                let path_str = mod_name(&args[0]);
                if path_str.is_empty() {
                    return Err(PyError::type_error(
                        "as_file() requires traversable with a valid name",
                    ));
                }
                let cm = create_module(
                    "_CtxManager",
                    HashMap::from([("name".to_string(), py_str(&path_str))]),
                );
                // Add __enter__/__exit__ as BuiltinMethod with self_obj = cm
                // so that when called via module.__enter__(), the function receives
                // the module itself as args[0] (via BuiltinMethod self-binding).
                if let PyObject::Module { dict, .. } = &mut *cm.borrow_mut() {
                    dict.insert_str(
                        "__enter__",
                        PyObjectRef::new(PyObject::BuiltinMethod {
                            name: "__enter__".to_string(),
                            func: enter_cm,
                            self_obj: cm.clone(),
                        }),
                    );
                    dict.insert_str(
                        "__exit__",
                        PyObjectRef::new(PyObject::BuiltinMethod {
                            name: "__exit__".to_string(),
                            func: exit_cm,
                            self_obj: cm.clone(),
                        }),
                    );
                }
                Ok(cm)
            },
        }),
    );

    // files(package) -> traversable with joinpath()
    d.insert_str(
        "files",
        PyObjectRef::new(PyObject::BuiltinFunction {
            name: "files".to_string(),
            func: |args| {
                if args.is_empty() {
                    return Err(PyError::type_error(
                        "files() requires 1 argument (package name)",
                    ));
                }
                let pkg_name = args[0].str();
                // Look up the package's __path__ via VM_PTR
                let pkg_path: String = with_vm_mut(|vm| -> PyResult<String> {
                    match vm.modules.get(&pkg_name) {
                        Some(mod_obj) => {
                            let borrowed = mod_obj.borrow();
                            if let PyObject::Module { dict, .. } = &*borrowed {
                                if let Some(path_list) = dict.get_str("__path__") {
                                    if let PyObject::List(items) = &*path_list.borrow() {
                                        if let Some(first) = items.first() {
                                            if let PyObject::Str(s) = &*first.borrow() {
                                                Ok(s.to_string())
                                            } else {
                                                Ok(format!("./{}", pkg_name))
                                            }
                                        } else {
                                            Ok(format!("./{}", pkg_name))
                                        }
                                    } else {
                                        Ok(format!("./{}", pkg_name))
                                    }
                                } else {
                                    Ok(format!("./{}", pkg_name))
                                }
                            } else {
                                Ok(format!("./{}", pkg_name))
                            }
                        }
                        None => Ok(format!("./{}", pkg_name)),
                    }
                })??;

                let trav = create_module(
                    "_Traversable",
                    HashMap::from([("name".to_string(), py_str(&pkg_path))]),
                );
                // Add joinpath as BuiltinMethod with self_obj = trav
                // so that when called via traversable.joinpath(...), the function receives
                // the traversable itself as args[0] (via BuiltinMethod self-binding).
                if let PyObject::Module { dict, .. } = &mut *trav.borrow_mut() {
                    dict.insert_str(
                        "joinpath",
                        PyObjectRef::new(PyObject::BuiltinMethod {
                            name: "joinpath".to_string(),
                            func: trav_joinpath,
                            self_obj: trav.clone(),
                        }),
                    );
                }
                // __str__ via name attribute
                Ok(trav)
            },
        }),
    );

    d
}
