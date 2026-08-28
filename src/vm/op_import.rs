use crate::bytecode::Opcode;
use crate::interner;
use crate::object::*;
use crate::vm::VirtualMachine;
use num_traits::ToPrimitive;

impl VirtualMachine {
    pub(crate) fn handle_import(&mut self, fi: usize, op: Opcode, arg: u32) -> PyResult<bool> {
        match op {
            Opcode::IMPORT_NAME => {
                let name_idx = arg as usize;
                let name = crate::interner::lookup(self.frames[fi].code.names[name_idx]);
                // Pop level (int, TOS) and fromlist (TOS1)
                let level_val = self.frames[fi].pop()?;
                let _fromlist = self.frames[fi].pop()?;
                // Resolve relative imports: if level > 0, use __package__ from frame globals
                let resolved = {
                    let level = {
                        let obj = level_val.borrow();
                        match &*obj {
                            PyObject::Int(i) => i.to_i64().unwrap_or(0) as usize,
                            _ => 0,
                        }
                    };
                    if level > 0 {
                        let pkg = self.frames[fi]
                            .globals
                            .borrow()
                            .get(&interner::intern("__package__"))
                            .cloned()
                            .and_then(|p| {
                                let p = p.borrow();
                                if let PyObject::Str(s) = &*p {
                                    Some(s.to_string())
                                } else {
                                    None
                                }
                            });
                        // level=1 (`from . import x`) resolves relative to
                        // __package__ itself; each additional dot
                        // (level=2 → `from .. import x`, etc.) goes up one
                        // more enclosing package, stripping one more
                        // trailing component. This was previously ignored
                        // entirely — `from .. import x` resolved identically
                        // to `from . import x`, silently importing from the
                        // wrong (child, not parent) package whenever a
                        // module used a multi-dot relative import.
                        let pkg = pkg.map(|p| {
                            let mut segs: Vec<&str> = p.split('.').collect();
                            let strip = level.saturating_sub(1);
                            if strip >= segs.len() {
                                segs.clear();
                            } else {
                                segs.truncate(segs.len() - strip);
                            }
                            segs.join(".")
                        });
                        let resolved_name = match pkg {
                            Some(p) if !p.is_empty() => {
                                if name.is_empty() {
                                    p
                                } else {
                                    format!("{}.{}", p, name)
                                }
                            }
                            // Fallback: use __name__ up to last dot as package
                            _ => {
                                let n = self.frames[fi]
                                    .globals
                                    .borrow()
                                    .get(&interner::intern("__name__"))
                                    .cloned()
                                    .and_then(|n| {
                                        let n = n.borrow();
                                        if let PyObject::Str(s) = &*n {
                                            Some(s.to_string())
                                        } else {
                                            None
                                        }
                                    })
                                    .unwrap_or_default();
                                if let Some(dot) = n.rfind('.') {
                                    let base = &n[..dot];
                                    if name.is_empty() {
                                        base.to_string()
                                    } else {
                                        format!("{}.{}", base, name)
                                    }
                                } else {
                                    name.clone()
                                }
                            }
                        };
                        resolved_name
                    } else {
                        name.clone()
                    }
                };
                if std::env::var("RPY_DEBUG_IMPORT").is_ok() {
                    eprintln!(
                        "IMPORT_NAME: resolved={} cached={}",
                        resolved,
                        self.modules.contains_key(&resolved)
                    );
                }
                if let Some(module) = self.import_cached_or_fresh(&resolved) {
                    // For 'import a.b.c' where fromlist is empty (regular import, not 'from a.b import X'),
                    // push the top-level module so STORE_NAME stores the package, not the submodule
                    let is_from_import = {
                        let obj = _fromlist.borrow();
                        matches!(&*obj, PyObject::Tuple(items) if !items.is_empty())
                    };
                    if resolved.contains('.') && !is_from_import {
                        // Set sub-module as attribute on parent module (e.g. logging.config = <module>)
                        if let Some(dot_pos) = resolved.rfind('.') {
                            let parent_name = &resolved[..dot_pos];
                            let child_name = &resolved[dot_pos + 1..];
                            if let Some(parent_mod) = self.modules.get(parent_name) {
                                let _ = parent_mod
                                    .borrow_mut()
                                    .set_attribute(child_name, module.clone());
                            }
                        }
                        if let Some(top) = resolved.split('.').next() {
                            if let Some(top_mod) = self.modules.get(top) {
                                self.frames[fi].push(top_mod.clone());
                            } else {
                                self.frames[fi].push(module.clone());
                            }
                        } else {
                            self.frames[fi].push(module.clone());
                        }
                    } else {
                        self.frames[fi].push(module.clone());
                    }
                } else {
                    match self.import_module_from_file(&resolved) {
                        Ok(module) => {
                            self.modules.insert(resolved.clone(), module.clone());
                            // Register in sys.modules (safe: module fully loaded)
                            if let Some(sys_mod) = self.modules.get("sys") {
                                if let PyObject::Module { dict, .. } = &*sys_mod.borrow() {
                                    if let Some(md) = dict.get_str("modules").cloned() {
                                        match &md {
                                            PyObjectRef::Mut(rc) => {
                                                if let Ok(mut guard) = rc.try_borrow_mut() {
                                                    if let PyObject::Dict(ref mut d) = &mut *guard {
                                                        d.set(py_str(&resolved), module.clone())
                                                            .ok();
                                                    }
                                                }
                                            }
                                            _ => {}
                                        }
                                    }
                                }
                            }
                            self.frames[fi].push(module);
                            // For 'import a.b.c' where fromlist is empty,
                            // push top-level module instead of deepest module
                            let is_from_import = {
                                let obj = _fromlist.borrow();
                                matches!(&*obj, PyObject::Tuple(items) if !items.is_empty())
                            };
                            if resolved.contains('.') && !is_from_import {
                                if let Some(top) = resolved.split('.').next() {
                                    if let Some(top_mod) = self.modules.get(top) {
                                        let _ = self.frames[fi].pop();
                                        self.frames[fi].push(top_mod.clone());
                                    }
                                }
                            }
                        }
                        Err(e) => return Err(e),
                    }
                }
            }

            Opcode::IMPORT_FROM => {
                let name_idx = arg as usize;
                let name = crate::interner::lookup(self.frames[fi].code.names[name_idx]);
                let module = self.frames[fi].peek(0)?;
                // Handle 'from module import *' — when the imported name is '*',
                // iterate over the module's dict and store all names in current scope
                if name == "*" {
                    let module_borrowed = module.borrow();
                    if let PyObject::Module { dict, .. } = &*module_borrowed {
                        // Use __all__ if present, otherwise all non-underscore names
                        let names_to_import: Vec<String> =
                            if let Some(all_val) = dict.get_str("__all__") {
                                let all_borrowed = all_val.borrow();
                                match &*all_borrowed {
                                    PyObject::Tuple(items) | PyObject::List(items) => items
                                        .iter()
                                        .filter_map(|n| {
                                            if let PyObject::Str(s) = &*n.borrow() {
                                                Some(s.to_string())
                                            } else {
                                                None
                                            }
                                        })
                                        .collect(),
                                    _ => dict
                                        .keys()
                                        .map(|k| interner::lookup_str(*k))
                                        .filter(|k| !k.starts_with('_'))
                                        .map(|k| k.to_string())
                                        .collect(),
                                }
                            } else {
                                dict.keys()
                                    .map(|k| interner::lookup_str(*k))
                                    .filter(|k| !k.starts_with('_'))
                                    .map(|k| k.to_string())
                                    .collect()
                            };
                        // Collect name-value pairs before dropping borrow
                        let imports: Vec<(String, PyObjectRef)> = names_to_import
                            .iter()
                            .filter_map(|name| {
                                dict.get_str(&name).map(|val| (name.clone(), val.clone()))
                            })
                            .collect();
                        drop(module_borrowed);
                        let live_module = self.frames[fi].live_module.clone();
                        for (import_name, val) in &imports {
                            if let Some(order) = self.frames[fi].name_order.clone() {
                                let mut order = order.borrow_mut();
                                if !order.contains(import_name) {
                                    order.push(import_name.clone());
                                }
                            }
                            if let Some(lm) = &live_module {
                                if let PyObject::Module { dict, .. } = &mut *lm.borrow_mut() {
                                    dict.insert_str(import_name, val.clone());
                                }
                            }
                            self.frames[fi]
                                .globals
                                .borrow_mut()
                                .insert(interner::intern(&import_name), val.clone());
                        }
                        // Push placeholder module result (the loop above already pushed values)
                        // The POP_TOP after IMPORT_FROM loop will clean up
                        self.frames[fi].push(py_none());
                        return Ok(true);
                    }
                }
                // Check if name is in module's dict first (without holding borrow)
                let found = {
                    let obj = module.borrow();
                    match &*obj {
                        PyObject::Module { dict, .. } => dict.get_str(&name).cloned(),
                        _ => return Err(PyError::runtime_error("IMPORT_FROM on non-module")),
                    }
                };
                // Get module name for submodule import (clone to avoid borrow conflicts)
                let module_name = {
                    let obj = module.borrow();
                    match &*obj {
                        PyObject::Module { name: mn, .. } => mn.clone(),
                        _ => return Err(PyError::runtime_error("IMPORT_FROM on non-module")),
                    }
                };
                // Circular-import fallback: if this module is STILL mid-execution
                // further down the call stack (e.g. its __init__.py does
                // `import package.submodule` as its last statement, and that
                // submodule does `from . import name_defined_earlier`), the
                // module object's own dict is only populated once the whole
                // body finishes — it's a snapshot copy, not a live view of the
                // executing frame's globals. Check ancestor frames' actual
                // live globals for the name before giving up.
                let found_direct = found.is_some();
                let found = found.or_else(|| {
                    self.frames.iter().find_map(|f| {
                        let g = f.globals.borrow();
                        if g.get(&interner::intern("__name__"))
                            .map(|n| n.str())
                            .as_deref()
                            == Some(module_name.as_str())
                        {
                            g.get(&interner::intern(&name)).cloned()
                        } else {
                            None
                        }
                    })
                });
                if std::env::var("RPY_DEBUG_IMPORT").is_ok() {
                    eprintln!(
                        "IMPORT_FROM: name={} module={} found_direct={} found_after_ancestor={}",
                        name,
                        module_name,
                        found_direct,
                        found.is_some()
                    );
                }
                if let Some(val) = found {
                    self.frames[fi].push(val);
                } else {
                    // Try importing as sub-module (for dotted names like os.path)
                    let submodule_name = format!("{}.{}", module_name, name);
                    if std::env::var("RPY_DEBUG_IMPORT").is_ok() {
                        eprintln!(
                            "IMPORT_FROM fallback: submodule_name={} already_cached={}",
                            submodule_name,
                            self.modules.contains_key(&submodule_name)
                        );
                    }
                    if submodule_name.contains('.') {
                        match self.import_module_from_file(&submodule_name) {
                            Ok(submod) => {
                                self.modules.insert(submodule_name.clone(), submod.clone());
                                if let PyObject::Module { dict, .. } = &mut *module.borrow_mut() {
                                    dict.insert_str(&name, submod.clone());
                                }
                                self.frames[fi].push(submod);
                            }
                            Err(e) => {
                                // RPY_DEBUG_IMPORT=1 prints the real underlying
                                // error before it potentially gets flattened
                                // into the generic message below (kept
                                // permanently per user request, 2026-07-19).
                                if std::env::var("RPY_DEBUG_IMPORT").is_ok() {
                                    eprintln!(
                                        "IMPORT_FROM_FAIL: name={} module={} err={}",
                                        name, module_name, e
                                    );
                                }
                                // Only "the submodule doesn't exist" collapses
                                // to CPython's generic "cannot import name"
                                // message here. Any other error (e.g. the
                                // submodule's own body raising a real
                                // exception) must propagate as that real
                                // exception — previously this branch always
                                // discarded it in favor of the generic
                                // ImportError, so a genuine bug inside an
                                // imported submodule was indistinguishable
                                // from the name simply not existing.
                                if matches!(e, PyError::ImportError(_)) {
                                    return Err(PyError::ImportError(format!(
                                        "cannot import name '{}' from '{}'",
                                        name, module_name
                                    )));
                                }
                                return Err(e);
                            }
                        }
                    } else {
                        return Err(PyError::ImportError(format!(
                            "cannot import name '{}' from '{}'",
                            name, module_name
                        )));
                    }
                }
            }

            Opcode::IMPORT_STAR => {
                // `from x import *` — copy every public (non-underscore,
                // subject to `__all__`) name from the module into the current
                // namespace (test_pkg::test_2). The module is on the stack.
                let module = self.frames[fi].pop()?;
                let names: Vec<String> = {
                    let obj = module.borrow();
                    match &*obj {
                        PyObject::Module { dict, .. } => {
                            if let Some(all) = dict.get_str("__all__").cloned() {
                                match &*all.borrow() {
                                    PyObject::List(items) | PyObject::Tuple(items) => {
                                        items.iter().map(|i| i.str()).collect()
                                    }
                                    _ => Vec::new(),
                                }
                            } else {
                                let mut v: Vec<String> = dict
                                    .iter()
                                    .map(|(k, _)| interner::lookup_str(*k))
                                    .filter(|k| !k.starts_with('_'))
                                    .map(|k| k.to_string())
                                    .collect();
                                v.sort();
                                v
                            }
                        }
                        _ => return Err(PyError::runtime_error("IMPORT_STAR on non-module")),
                    }
                };
                let mut g = self.frames[fi].globals.borrow_mut();
                for n in &names {
                    let obj = module.borrow();
                    if let PyObject::Module { dict, .. } = &*obj {
                        if let Some(v) = dict.get(&interner::intern(n)).cloned() {
                            g.insert(interner::intern(n), v);
                        }
                    }
                }
                drop(g);
                // Also mirror into frame.locals so subsequent LOAD_NAME in
                // the same scope sees the names (exec with separate locals).
                let mut locals = self.frames[fi].locals.clone();
                for n in &names {
                    let obj = module.borrow();
                    if let PyObject::Module { dict, .. } = &*obj {
                        if let Some(v) = dict.get(&interner::intern(n)).cloned() {
                            locals.insert(interner::intern(n), v);
                        }
                    }
                }
            }

            Opcode::LOAD_BUILD_CLASS => {
                self.frames[fi].push(PyObjectRef::imm(PyObject::BuildClass));
            }

            Opcode::LOAD_CLOSURE => {
                let idx = arg as usize;
                let cell = {
                    let f = &self.frames[self.frames.len() - 1];
                    if idx < f.code.cellvars.len() {
                        let name = &f.code.cellvars[idx];
                        if let Some(var_idx) = f
                            .code
                            .varnames
                            .iter()
                            .position(|&n| crate::interner::intern_eq(n, name))
                        {
                            if let Some(val) = f.fast_locals.get(var_idx).and_then(|v| v.clone()) {
                                val
                            } else {
                                PyObjectRef::new(PyObject::Cell { value: None })
                            }
                        } else {
                            PyObjectRef::new(PyObject::Cell { value: None })
                        }
                    } else {
                        let fv_idx = idx - f.code.cellvars.len();
                        if let Some(cell) = f.closure.get(fv_idx).cloned() {
                            cell
                        } else {
                            PyObjectRef::new(PyObject::Cell { value: None })
                        }
                    }
                };
                self.frames[fi].push(cell);
            }

            _ => return Ok(false),
        }
        Ok(true)
    }
}
