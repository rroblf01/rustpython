use crate::bytecode::*;
use crate::compiler::Compiler;
use crate::interner::{self, StrId};
use crate::object::*;
use crate::parser::Parser;
use crate::vm::VirtualMachine;
use crate::vm::helpers::{eval_const_value, find_lib_dir};
use smallvec::SmallVec;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

impl VirtualMachine {
    /// Return the cached module for `name` if it's genuinely still imported
    /// (`sys.modules` has it). If it was `del sys.modules['x']`'d, build a
    /// FRESH module object (sharing the dict contents) and re-register it in
    /// both maps — real Python re-imports the module, and for a native module
    /// a fresh object is the faithful equivalent (test_atexit's
    /// test_atexit_instances asserts `atexit2 is not atexit1` while both
    /// share the same callback registry).

    pub fn import_module_from_file(&mut self, name: &str) -> PyResult<PyObjectRef> {
        // Guard against genuine infinite import recursion with a clean
        // error (showing the exact chain) instead of a raw stack overflow —
        // kept permanently (env-gated print is always-on; the depth check
        // itself is cheap) rather than added back by hand each time.
        thread_local! {
            static IMPORT_CHAIN: RefCell<Vec<String>> = RefCell::new(Vec::new());
        }
        let depth = IMPORT_CHAIN.with(|c| c.borrow().len());
        if depth > 150 {
            let chain = IMPORT_CHAIN.with(|c| c.borrow().join(" -> "));
            return Err(PyError::ImportError(format!(
                "import recursion too deep, likely a genuine cycle: {} -> {}",
                chain, name
            )));
        }
        IMPORT_CHAIN.with(|c| c.borrow_mut().push(name.to_string()));
        if std::env::var("RPY_DEBUG_IMPORT").is_ok() {
            eprintln!(
                "{}IMPORT_FILE: {} (self.modules.len()={}, sys.path={:?})",
                "  ".repeat(depth),
                name,
                self.modules.len(),
                self.modules.get("sys").and_then(|m| {
                    if let PyObject::Module { dict, .. } = &*m.borrow() {
                        dict.get_str("path").map(|p| p.str())
                    } else {
                        None
                    }
                })
            );
        }
        let result = self.import_module_from_file_inner(name);
        IMPORT_CHAIN.with(|c| {
            c.borrow_mut().pop();
        });
        result
    }

    fn import_module_from_file_inner(&mut self, name: &str) -> PyResult<PyObjectRef> {
        if cfg!(feature = "profile") {
            if let Ok(status) =
                std::fs::read_to_string(format!("/proc/{}/status", std::process::id()))
            {
                if let Some(_rss_line) = status.lines().find(|l| l.starts_with("VmRSS:")) {}
                if let Some(_peak_line) = status.lines().find(|l| l.starts_with("VmPeak:")) {}
            }
        }
        // Handle dotted names: e.g. "certifi.core" or "django.utils.version"
        // Walk through each segment, importing missing packages as we go
        if let Some(_dot_pos) = name.find('.') {
            let parts: Vec<&str> = name.split('.').collect();
            let mut current_name = parts[0].to_string();
            let mut parent_path: Option<Vec<String>> = None;

            // A multi-part dotted import (e.g. `import django.template.engine`)
            // must initialize each ancestor package in order first, matching
            // real Python's import semantics. Without this, when `django`
            // isn't already cached, the code below falls through to a
            // direct full-path file lookup ("django/template/engine.py")
            // that finds the leaf file directly, silently skipping every
            // intermediate package's __init__.py — including module-level
            // side effects (signal registration, singletons like
            // `engines = EngineHandler()`) that code loaded later
            // transitively depends on already having run.
            if !self.modules.contains_key(&current_name) {
                let _ = self.import_module_from_file(&current_name);
            }
            // Check if we already have the top-level module
            if !self.modules.contains_key(&current_name) {
                if cfg!(feature = "profile") {
                    eprintln!("DEBUG import: top-level '{}' NOT in modules", current_name);
                }
                // Not in modules — fall through to regular file search below
            } else {
                // Walk the chain: for each part after the first, resolve the child
                let mut all_resolved = true;
                for i in 1..parts.len() {
                    let child = parts[i];
                    let full_name = format!("{}.{}", current_name, child);

                    // If already in modules, skip to next
                    if self.modules.contains_key(&full_name) {
                        current_name = full_name;
                        parent_path = None;
                        continue;
                    }

                    // Get the parent's __path__ (all entries, not just first)
                    if parent_path.is_none() {
                        if let Some(parent_mod) = self.modules.get(&current_name) {
                            let borrowed = parent_mod.borrow();
                            if let PyObject::Module { dict, .. } = &*borrowed {
                                let p = dict.get_str("__path__").and_then(|pl| {
                                    if let PyObject::List(items) = &*pl.borrow() {
                                        let paths: Vec<String> = items
                                            .iter()
                                            .filter_map(|i| {
                                                if let PyObject::Str(s) = &*i.borrow() {
                                                    Some(s.to_string())
                                                } else {
                                                    None
                                                }
                                            })
                                            .collect();
                                        if paths.is_empty() { None } else { Some(paths) }
                                    } else {
                                        None
                                    }
                                });
                                parent_path = p;
                            } else {
                                parent_path = None;
                            }
                        } else {
                            parent_path = None;
                        }
                    }

                    // Try to find the child as a file/subpackage in parent's __path__
                    if let Some(ref bases) = parent_path {
                        let mut found_child = false;
                        'outer: for base in bases {
                            let base_trimmed = base.trim_end_matches('/');
                            for candidate in &[
                                format!("{}/{}.py", base_trimmed, child),
                                format!("{}/{}/__init__.py", base_trimmed, child),
                            ] {
                                if let Some(source) = self.read_module_source(candidate)? {
                                found_child = true;
                                let is_pkg = candidate.ends_with("__init__.py");
                                let empty_dict = if is_pkg {
                                    if let Some(pkg_dir) = std::path::Path::new(candidate).parent()
                                    {
                                        HashMap::from([
                                            (
                                                "__path__".to_string(),
                                                py_list(vec![py_str(
                                                    &pkg_dir.to_string_lossy().to_string(),
                                                )]),
                                            ),
                                            ("__package__".to_string(), py_str(&full_name)),
                                        ])
                                    } else {
                                        HashMap::new()
                                    }
                                } else {
                                    HashMap::new()
                                };
                                let empty_mod = create_module(&full_name, empty_dict);
                                self.modules.insert(full_name.clone(), empty_mod.clone());
                                // Register in sys.modules BEFORE executing (needed by code that checks sys.modules[__name__])
                                // Using cloned PyObjectRef to avoid holding borrow across exec_module_source
                                let sys_modules = self.modules.get("sys").and_then(|m| {
                                    let b = m.borrow();
                                    match &*b {
                                        PyObject::Module { dict, .. } => {
                                            dict.get_str("modules").cloned()
                                        }
                                        _ => None,
                                    }
                                });
                                if let Some(sm) = sys_modules {
                                    // Use try_borrow_mut to avoid RefCell panic if already borrowed
                                    match &sm {
                                        PyObjectRef::Mut(rc) => {
                                            if let Ok(mut guard) = rc.try_borrow_mut() {
                                                if let PyObject::Dict(ref mut d) = &mut *guard {
                                                    d.set(py_str(&full_name), empty_mod.clone())
                                                        .ok();
                                                }
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                                // Execute the module source
                                let module =
                                    self.exec_module_source(&source, candidate, &full_name)?;
                                self.modules.insert(full_name.clone(), module.clone());
                                // Wire into parent module namespace
                                if let Some(dot_pos) = full_name.rfind('.') {
                                    let parent_name = full_name[..dot_pos].to_string();
                                    let child_name = full_name[dot_pos + 1..].to_string();
                                    if let Some(parent_mod) =
                                        self.modules.get(&parent_name).cloned()
                                    {
                                        if let PyObject::Module { dict, .. } =
                                            &mut *parent_mod.borrow_mut()
                                        {
                                            dict.insert_str(&child_name, module.clone());
                                        }
                                    }
                                }
                                current_name = full_name;
                                parent_path = None;
                                break 'outer;
                            }
                            }
                            }
                        if !found_child {
                            // Neither `child.py` nor `child/__init__.py` exists
                            // under the parent's __path__ — this dotted
                            // component isn't a submodule (could be a plain
                            // attribute of the parent, e.g. `from pkg import
                            // some_function`, or genuinely missing). Previously
                            // falling through here silently left `current_name`
                            // pointing at the last-resolved ANCESTOR package,
                            // and `all_resolved` stayed true, so the caller
                            // returned that ancestor module mislabeled as the
                            // full dotted name — e.g. `import pkg.missing`
                            // would silently succeed, yielding `pkg` itself.
                            all_resolved = false;
                            break;
                        }
                    } else {
                        all_resolved = false;
                        break;
                    }
                }
                if all_resolved {
                    if let Some(result) = self.modules.get(&current_name).cloned() {
                        return Ok(result);
                    }
                }
                // If we resolved some but not all, continue to search
                // from the last unresolved parent
            }

            // If we didn't have the top-level or couldn't walk the chain,
            // fall through to regular sys.path search below
        }

        // Search sys.path for the module
        let search_paths = self.get_sys_path();
        let py_name = name.replace('.', "/");
        for base in &search_paths {
            let py_path = if base.ends_with('/') {
                format!("{}{}.py", base, py_name)
            } else {
                format!("{}/{}.py", base, py_name)
            };
            if let Some(source) = self.read_module_source(&py_path)? {
                let empty_mod = create_module(name, HashMap::new());
                self.modules.insert(name.to_string(), empty_mod.clone());
                if let Some(sys_mod) = self.modules.get("sys") {
                    if let PyObject::Module { dict, .. } = &*sys_mod.borrow() {
                        if let Some(mod_dict) = dict.get_str("modules").cloned() {
                            mod_dict
                                .borrow_mut()
                                .set_attribute(name, empty_mod.clone())
                                .ok();
                        }
                    }
                }
                let module = self.exec_module_source(&source, &py_path, name)?;
                self.modules.insert(name.to_string(), module.clone());
                // Wire submodule into parent module namespace and update sys.modules
                if let Some(sys_mod) = self.modules.get("sys") {
                    if let PyObject::Module { dict, .. } = &*sys_mod.borrow() {
                        if let Some(mod_dict) = dict.get_str("modules").cloned() {
                            mod_dict
                                .borrow_mut()
                                .set_attribute(name, module.clone())
                                .ok();
                        }
                    }
                }
                // Wire submodule into parent module namespace
                if let Some(dot_pos) = name.rfind('.') {
                    let parent_name = name[..dot_pos].to_string();
                    let child_name = name[dot_pos + 1..].to_string();
                    if let Some(parent_mod) = self.modules.get(&parent_name).cloned() {
                        if let PyObject::Module { dict, .. } = &mut *parent_mod.borrow_mut() {
                            dict.insert_str(&child_name, module.clone());
                        }
                    }
                }
                return Ok(module);
            }
            let init_path = if base.ends_with('/') {
                format!("{}{}/__init__.py", base, py_name)
            } else {
                format!("{}/{}/__init__.py", base, py_name)
            };
            if let Some(source) = self.read_module_source(&init_path)? {
                let pkg_dir = std::path::Path::new(&init_path)
                    .parent()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default();
                let mut empty_dict = HashMap::new();
                empty_dict.insert_str("__path__", py_list(vec![py_str(&pkg_dir)]));
                empty_dict.insert_str("__package__", py_str(name));
                let empty_mod = create_module(name, empty_dict);
                self.modules.insert(name.to_string(), empty_mod.clone());
                if let Some(sys_mod) = self.modules.get("sys") {
                    if let PyObject::Module { dict, .. } = &*sys_mod.borrow() {
                        if let Some(mod_dict) = dict.get_str("modules").cloned() {
                            mod_dict
                                .borrow_mut()
                                .set_attribute(name, empty_mod.clone())
                                .ok();
                        }
                    }
                }
                let module = self.exec_module_source(&source, &init_path, name)?;
                self.modules.insert(name.to_string(), module.clone());
                // Update sys.modules with the loaded module (overwrites empty stub)
                if let Some(sys_mod) = self.modules.get("sys") {
                    if let PyObject::Module { dict, .. } = &*sys_mod.borrow() {
                        if let Some(mod_dict) = dict.get_str("modules").cloned() {
                            mod_dict
                                .borrow_mut()
                                .set_attribute(name, module.clone())
                                .ok();
                        }
                    }
                }
                return Ok(module);
            }
            // Try loading as a .so C extension (requires the "ffi" feature)
            #[cfg(feature = "ffi")]
            {
                let so_path = if base.ends_with('/') {
                    format!("{}{}.cpython-313-x86_64-linux-gnu.so", base, name)
                } else {
                    format!("{}/{}.cpython-313-x86_64-linux-gnu.so", base, name)
                };
                if std::path::Path::new(&so_path).exists() {
                    // SAFETY: loading and running a CPython C extension's
                    // PyInit_* entry point is inherently unsafe — there is no
                    // way to verify the .so at `so_path` actually implements
                    // the CPython C-API contract it claims to. This is the
                    // deliberate, documented risk of the "ffi" feature: it
                    // only runs when the caller opts in by enabling it and
                    // pointing sys.path at a real compiled extension.
                    let loaded = unsafe { crate::ffi_bridge::load_extension(&so_path, name) };
                    match loaded {
                        Ok(()) => {
                            // Try to get the module from the extension registry
                            // SAFETY: see above — same trust boundary, reading
                            // state populated by the load_extension call just above.
                            if let Some(mod_obj) =
                                unsafe { crate::ffi_bridge::get_extension_module(name) }
                            {
                                return Ok(mod_obj);
                            }
                        }
                        Err(_) => {}
                    }
                }
            }
        }
        Err(PyError::module_not_found_error(format!(
            "No module named '{}'",
            name
        )))
    }

    fn get_sys_path(&self) -> Vec<String> {
        if let Some(sys_mod) = self.modules.get("sys") {
            if let PyObject::Module { dict, .. } = &*sys_mod.borrow() {
                if let Some(path_list) = dict.get_str("path") {
                    if let PyObject::List(items) = &*path_list.borrow() {
                        return items
                            .iter()
                            .filter_map(|item| {
                                if let PyObject::Str(s) = &*item.borrow() {
                                    Some(s.to_string())
                                } else {
                                    None
                                }
                            })
                            .collect();
                    }
                }
            }
        }
        vec![]
    }

    /// Read a `.py` source file off disk and decode it as real Python would:
    /// PEP 263 coding cookie if present, strict UTF-8 otherwise. Returns
    /// `Ok(None)` when the file simply doesn't exist (caller continues its
    /// path search), but PROPAGATES a decode failure — a file that exists
    /// yet isn't valid in its implied encoding is a `SyntaxError`, exactly
    /// as a real import would raise, NOT a silent "module not found".
    /// Deliberately NOT `std::fs::read_to_string` — that lossy-substitutes
    /// non-UTF-8 bytes (confirmed via `test_utf8source.py::test_badsyntax`).
    fn read_module_source(&self, path: &str) -> PyResult<Option<String>> {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(_) => return Ok(None),
        };
        crate::object::import_builtin::decode_source_bytes(&bytes).map(Some)
    }

    fn exec_module_source(
        &mut self,
        source: &str,
        path: &str,
        name: &str,
    ) -> PyResult<PyObjectRef> {
        // ── .pyc cache support ─────────────────────────────────────────
        // Try to load a previously-compiled .pyc file. If valid (matching
        // magic + version + source timestamp), skip parsing and compilation.
        const PYC_MAGIC: u32 = 0x52535079; // "RSPy"
        const PYC_VERSION: u16 = 2; // bumped: CodeObject gained kwonly_defaults_mask

        let py_path = std::path::Path::new(path);
        let source_mtime = std::fs::metadata(path)
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let mut cached_code: Option<CodeObject> = None;

        // Compute __pycache__/basename.pyc path
        if let Some(parent) = py_path.parent() {
            if let Some(stem) = py_path.file_stem().and_then(|s| s.to_str()) {
                let pyc_dir = parent.join("__pycache__");
                let pyc_filename = format!("{}.rustpython-0.pyc", stem);
                let pyc_path = pyc_dir.join(&pyc_filename);

                if let Ok(pyc_data) = std::fs::read(&pyc_path) {
                    // Minimum size: magic(4) + version(2) + timestamp(8) = 14 bytes
                    if pyc_data.len() >= 14 {
                        let magic = u32::from_le_bytes([
                            pyc_data[0],
                            pyc_data[1],
                            pyc_data[2],
                            pyc_data[3],
                        ]);
                        let version = u16::from_le_bytes([pyc_data[4], pyc_data[5]]);
                        let ts = u64::from_le_bytes([
                            pyc_data[6],
                            pyc_data[7],
                            pyc_data[8],
                            pyc_data[9],
                            pyc_data[10],
                            pyc_data[11],
                            pyc_data[12],
                            pyc_data[13],
                        ]);
                        if magic == PYC_MAGIC && version == PYC_VERSION && ts == source_mtime {
                            if let Ok(code) = CodeObject::from_bytes(&pyc_data[14..]) {
                                cached_code = Some(code);
                            }
                        }
                    }
                }
            }
        }

        // Parse and compile, or deserialise from cache
        let code: CodeObject = match cached_code {
            Some(cached) => cached,
            None => {
                let mut parser = crate::parser::Parser::new(source);
                let program = parser.parse_program().map_err(|e| {
                    crate::object::PyError::syntax_error_with_filename(e, path, source)
                })?;
                drop(parser); // Free parser memory (AST is now in `program`)

                let mut compiler = crate::compiler::Compiler::new();
                let compiled = compiler
                    .compile(&program, path)
                    .map_err(|e| {
                        crate::object::PyError::syntax_error_with_filename(e, path, source)
                    })?;
                drop(compiler); // Free compiler internal tables
                drop(program); // Free AST — CodeObject is now self-contained

                // Write .pyc cache for future imports (skip for stdlib modules).
                // Stdlib modules under /usr/ are stable + huge; serialising them
                // costs CPU + a temporary Vec<u8> allocation, and writing usually
                // fails silently anyway due to permissions on /usr/lib/__pycache__/.
                if !path.starts_with("/usr") {
                    if let Some(parent) = py_path.parent() {
                        if let Some(stem) = py_path.file_stem().and_then(|s| s.to_str()) {
                            let pyc_dir = parent.join("__pycache__");
                            let pyc_filename = format!("{}.rustpython-0.pyc", stem);
                            let pyc_path = pyc_dir.join(&pyc_filename);

                            let mut pyc_data = Vec::new();
                            pyc_data.extend_from_slice(&PYC_MAGIC.to_le_bytes());
                            pyc_data.extend_from_slice(&PYC_VERSION.to_le_bytes());
                            pyc_data.extend_from_slice(&source_mtime.to_le_bytes());
                            pyc_data.extend_from_slice(&compiled.to_bytes());

                            let _ = std::fs::create_dir_all(&pyc_dir);
                            let _ = std::fs::write(&pyc_path, &pyc_data);
                        }
                    }
                }

                compiled
            }
        };

        let is_package = path.ends_with("__init__.py");
        // `__builtins__` must be the SAME shared `builtins` module that
        // `import builtins` returns and that `LOAD_GLOBAL` consults — not a
        // fresh frozen copy. Otherwise mutations like `builtins.len = f`
        // (test_dynamic::test_modify_builtins) are invisible to code in the
        // imported module, since its `LOAD_GLOBAL` would read a different
        // object. The shared module lives in `self.modules["builtins"]`
        // (populated at VM construction).
        let builtins_module = self
            .modules
            .get("builtins")
            .cloned()
            .unwrap_or_else(|| {
                create_module(
                    "builtins",
                    self.builtins
                        .iter()
                        .map(|(k, v)| (interner::lookup_str(*k).to_string(), v.clone()))
                        .collect(),
                )
            });
        let mut globals_map: HashMap<StrId, PyObjectRef> = HashMap::from([
            (interner::intern("__name__"), py_str(name)),
            (interner::intern("__file__"), py_str(path)),
            (interner::intern("__builtins__"), builtins_module),
        ]);
        if is_package {
            if let Some(pkg_dir) = std::path::Path::new(path).parent() {
                let pkg_dir_str = pkg_dir.to_string_lossy().to_string();
                globals_map.insert(
                    interner::intern("__path__"),
                    py_list(vec![py_str(&pkg_dir_str)]),
                );
                globals_map.insert(interner::intern("__package__"), py_str(name));
            }
        } else {
            // For non-package modules, __package__ should be set to the parent package name
            // (e.g., "django.apps" for "django.apps.registry") so relative imports work
            let pkg = name.rfind('.').map(|dot| &name[..dot]).unwrap_or("");
            globals_map.insert(
                interner::intern("__package__"),
                if pkg.is_empty() {
                    py_str("")
                } else {
                    py_str(pkg)
                },
            );
        }
        let module_globals = Rc::new(RefCell::new(globals_map));
        crate::object::register_module_globals(name, Rc::clone(&module_globals));
        // Register module in sys.modules BEFORE executing (needed for sys.modules[__name__] checks)
        if let Some(sys_mod) = self.modules.get("sys") {
            if let PyObject::Module { dict, .. } = &*sys_mod.borrow() {
                if let Some(sm) = dict.get_str("modules").cloned() {
                    match &sm {
                        PyObjectRef::Mut(rc) => {
                            if let Ok(mut guard) = rc.try_borrow_mut() {
                                if let PyObject::Dict(ref mut d) = &mut *guard {
                                    d.set(
                                        py_str(name),
                                        py_str(&format!("<module '{}' (loaded)>", name)),
                                    )
                                    .ok();
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        // If the caller already registered a placeholder module object under
        // this name (the normal case — see import_module_from_file_inner),
        // mirror every STORE_NAME into its dict live, as execution happens,
        // instead of only after the whole body finishes (see
        // `Frame::live_module`'s doc comment). This is what lets a
        // circular import elsewhere see names this module already defined
        // even while it's still mid-execution — matching real CPython,
        // where `module.__dict__` IS the executing frame's globals, not a
        // separate snapshot.
        let live_module = self.modules.get(name).cloned();
        if let Some(lm) = &live_module {
            if let PyObject::Module { dict, .. } = &mut *lm.borrow_mut() {
                for (k, v) in module_globals.borrow().iter() {
                    dict.insert_str(interner::lookup_str(*k), v.clone());
                }
            }
        }
        // Preserve the real exception (type + object) raised by the module
        // body as-is — this used to be flattened into a formatted String via
        // `.map_err(|e| format!("{}", e))`, which meant a module raising e.g.
        // TypeError during import surfaced to the importer as a generic
        // ImportError with the TypeError's message glued into the text
        // instead of the actual TypeError, so `except TypeError` (or
        // anything other than a blanket `except ImportError`/`except
        // Exception`) around an import could never catch it.
        if std::env::var("RPY_DEBUG_IMPORT").is_ok() {
            eprintln!("MODULE_EXEC_START: {}", name);
        }
        self.exec_code_with_module(code, Some(Rc::clone(&module_globals)), live_module)?;
        if std::env::var("RPY_DEBUG_IMPORT").is_ok() {
            eprintln!("MODULE_EXEC_DONE: {}", name);
        }
        let globals_copy = module_globals.borrow().clone();
        // If a placeholder module was already registered under this name
        // (e.g. by import_module_from_file, to support circular imports),
        // populate it in place rather than returning a brand new object —
        // any reference a circular importer already grabbed a clone of
        // must see the final contents too, not just IMPORT_FROM's own
        // live-frame fallback (which only covers names accessed while
        // still mid-execution).
        if let Some(existing) = self.modules.get(name).cloned() {
            if let PyObject::Module { dict, .. } = &mut *existing.borrow_mut() {
                for (k, v) in globals_copy.iter() {
                    dict.insert_str(interner::lookup_str(*k), v.clone());
                }
            }
            return Ok(existing);
        }
        Ok(create_module(
            name,
            globals_copy
                .into_iter()
                .map(|(k, v)| (interner::lookup_str(k).to_string(), v))
                .collect(),
        ))
    }

    /// Try to execute a simple function without creating a Frame.
    /// Returns Some(result) if the function was simple enough, None otherwise.
    pub(crate) fn try_exec_simple(code: &CodeObject, args: &[PyObjectRef]) -> Option<PyResult<PyObjectRef>> {
        if code.vararg_name.is_some() || code.kwarg_name.is_some() || code.num_defaults > 0 {
            return None;
        }
        // Keyword-only params need the slow path's missing-kwonly validation
        // (`f(1,2,3)` on `def f(a,b,/,c,*,d,e)` must raise "missing ... d'
        // and 'e'", not silently leave them unbound).
        if code.kwonlyarg_count > 0 {
            return None;
        }
        // A wrong argument count must raise `TypeError` (see the slow path's
        // own validation just below in `call_function`) — this fast path
        // has no such check of its own, so falling through to the slow path
        // whenever the count doesn't match exactly is what makes that
        // validation actually apply to every call, not just the ones that
        // happen to miss this "simple function" fast path.
        if args.len() != code.arg_count {
            return None;
        }
        let instrs = &code.instructions;
        if instrs.is_empty() || instrs.len() > 12 {
            return None;
        }
        // Pre-allocate local variables from arguments
        let mut locals: Vec<Option<PyObjectRef>> = vec![None; code.varnames.len()];
        for (i, arg) in args.iter().enumerate() {
            if i < locals.len() {
                locals[i] = Some(arg.clone());
            }
        }
        let mut stack: SmallVec<[PyObjectRef; 8]> = SmallVec::new();
        let mut ip: usize = 0;
        let n_instrs = instrs.len();
        loop {
            if ip >= n_instrs {
                return None;
            }
            let instr = &instrs[ip];
            ip += 1;
            match instr.op {
                Opcode::LOAD_FAST => {
                    let idx = instr.arg as usize;
                    let val = locals.get(idx)?.clone()?;
                    stack.push(val);
                }
                Opcode::STORE_FAST => {
                    let idx = instr.arg as usize;
                    let val = stack.pop()?;
                    if idx < locals.len() {
                        locals[idx] = Some(val);
                    }
                }
                Opcode::LOAD_CONST => {
                    // Shares `eval_const_value`/`const_cache` with the main
                    // `execute_instruction` LOAD_CONST handler — this fast
                    // path (no-`Frame` "simple function" execution) used to
                    // have its OWN independent copy of the same parsing
                    // logic, uncached, and using `s.trim_start_matches('_')`
                    // (only strips LEADING underscores) instead of the main
                    // copy's `s.chars().filter(|&c| c != '_')` (strips ALL
                    // of them) — an inconsistency that would have mis-parsed
                    // a mid-literal digit separator like `1_000_000`
                    // differently depending on which path happened to
                    // execute a given function.
                    let const_idx = instr.arg as usize;
                    let cached = code
                        .const_cache
                        .borrow()
                        .get(const_idx)
                        .and_then(|c| c.clone());
                    let obj = if let Some(obj) = cached {
                        obj
                    } else {
                        let const_val = code.consts.get(const_idx)?.clone();
                        // Only Function/Complex/Bytes/Tuple/Code consts are
                        // NOT handled here (this fast path only ever runs
                        // for "simple" functions — see this fn's own doc
                        // comment — which realistically only ever load
                        // None/Bool/Int/Float/String constants); fall back
                        // to the slow path for anything else via `None`.
                        if !matches!(
                            const_val,
                            ConstValue::None
                                | ConstValue::Bool(_)
                                | ConstValue::Int(_)
                                | ConstValue::Float(_)
                                | ConstValue::String(_)
                        ) {
                            return None;
                        }
                        let obj = eval_const_value(const_val).ok()?;
                        let mut cache = code.const_cache.borrow_mut();
                        if cache.len() <= const_idx {
                            cache.resize(const_idx + 1, None);
                        }
                        cache[const_idx] = Some(obj.clone());
                        obj
                    };
                    stack.push(obj);
                }
                Opcode::BINARY_OP => {
                    let right = stack.pop()?;
                    let left = stack.pop()?;
                    let result = match instr.arg {
                        0 => py_add(&left, &right),
                        1 => py_sub(&left, &right),
                        2 => py_mul(&left, &right),
                        3 => py_div(&left, &right),
                        4 => py_floor_div(&left, &right),
                        5 => py_mod(&left, &right),
                        6 => py_pow(&left, &right),
                        7 => py_lshift(&left, &right),
                        8 => py_rshift(&left, &right),
                        9 => py_bit_or(&left, &right),
                        10 => py_bit_xor(&left, &right),
                        11 => py_bit_and(&left, &right),
                        13 => py_getitem(&left, &right),
                        _ => return None,
                    };
                    match result {
                        Ok(v) => stack.push(v),
                        Err(e) => return Some(Err(e)),
                    }
                }
                Opcode::COMPARE_OP => {
                    let right = stack.pop()?;
                    let left = stack.pop()?;
                    let result = py_compare(&left, &right, instr.arg);
                    match result {
                        Ok(v) => stack.push(v),
                        Err(e) => return Some(Err(e)),
                    }
                }
                Opcode::POP_JUMP_IF_FALSE => {
                    let val = stack.pop()?;
                    match val.try_truthy() {
                        Ok(t) => {
                            if !t {
                                ip = instr.arg as usize;
                            }
                        }
                        Err(e) => return Some(Err(e)),
                    }
                }
                Opcode::JUMP_FORWARD => {
                    ip = ip + instr.arg as usize;
                }
                Opcode::JUMP_BACKWARD => {
                    ip = ip - (instr.arg as usize + 1);
                }
                Opcode::RETURN_VALUE => return Some(Ok(stack.pop()?)),
                Opcode::LOAD_ATTR => {
                    let obj = stack.pop()?;
                    // A plain `Instance` needs the FULL LOAD_ATTR protocol
                    // (property/descriptor invocation, `__getattr__`
                    // fallback) — this fast path's own inline
                    // `get_attribute` call is the same "raw, non-invoking"
                    // lookup used elsewhere (see
                    // `get_attribute_with_properties`'s doc comment), so it
                    // silently returned an un-invoked `property` object
                    // instead of calling its getter, and never consulted
                    // `__getattr__` at all. Confirmed general via a
                    // self-recursive property getter (`return self.__bases__`
                    // as its OWN body) that should recurse until
                    // `RecursionError` but instead "resolved" in one step by
                    // handing back the bare descriptor. Bail to the slow,
                    // `Frame`-based path (which gets this right) whenever the
                    // receiver could plausibly need it.
                    if matches!(&*obj.borrow(), PyObject::Instance { .. }) {
                        return None;
                    }
                    let name_id = code.names[instr.arg as usize];
                    let name = crate::interner::lookup_str(name_id);
                    let val = obj.borrow().get_attribute(name);
                    match val {
                        Ok(v) => stack.push(v),
                        Err(e) => return Some(Err(e)),
                    }
                }
                _ => return None,
            }
        }
    }
}
