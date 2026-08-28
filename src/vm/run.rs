use crate::bytecode::CodeObject;
use crate::interner::{self, InternedMap, StrId};
use crate::object::*;
use crate::vm::VirtualMachine;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

impl VirtualMachine {
    pub fn run(&mut self, code: CodeObject) -> PyResult<PyObjectRef> {
        // Real CPython always has a `__main__` module in `sys.modules`
        // backed by the running script's own globals — `__import__("__main__")`
        // (which `unittest.main()` calls unconditionally via
        // `TestProgram.__init__`'s `self.module = __import__(module)`) relies
        // on this. Without it every `if __name__ == "__main__": unittest.main()`
        // trailer in a real CPython test file raised `ImportError: No module
        // named '__main__'` instead of actually running the tests. Reuse the
        // existing `live_module` mirroring machinery (same mechanism a
        // regular file-backed module import already uses) so STORE_NAME/
        // DELETE_NAME at top level keep this module's `dict` in sync as the
        // script executes, instead of only registering it once, empty then finished.
        let main_module = self
            .modules
            .entry("__main__".to_string())
            .or_insert_with(|| create_module("__main__", HashMap::new()))
            .clone();
        if let Some(sys_mod) = self.modules.get("sys") {
            if let PyObject::Module { dict, .. } = &*sys_mod.borrow() {
                if let Some(mod_dict) = dict.get_str("modules") {
                    // `sys.modules` is a real `dict` — `set_attribute` sets
                    // an OBJECT ATTRIBUTE (routed to `PyObject::Dict`'s own
                    // catch-all side-attribute-storage arm for non-Instance
                    // builtins), not a dict KEY, so this silently failed to
                    // make `"__main__"` actually appear via `sys.modules[...]`/
                    // `in sys.modules` at all — only `self.modules` (this
                    // VM's own Rust-side registry, which `import __main__`
                    // itself consults) ever really had it. Confirmed via the
                    // simplest repro: `import __main__` succeeds but
                    // `"__main__" in sys.modules` is `False` right after.
                    if let PyObject::Dict(d) = &mut *mod_dict.borrow_mut() {
                        let _ = d.set(py_str("__main__"), main_module.clone());
                    }
                }
            }
        }
        // JIT compilation disabled — using stable interpreter path only
        let mut frame = self.acquire_frame(
            Rc::new(code),
            self.globals.clone(),
            Rc::clone(&self.builtins),
            None,
        );
        frame.live_module = Some(main_module);
        self.push_frame(frame);
        let result = self.execute();
        if let Some(frame) = self.frames.pop() {
            self.release_frame(frame);
        }
        result
    }

    pub fn exec_code(
        &mut self,
        code: CodeObject,
        globals: Option<Rc<RefCell<HashMap<StrId, PyObjectRef>>>>,
    ) -> PyResult<PyObjectRef> {
        self.exec_code_with_module(code, globals, None)
    }

    /// Like `exec_code`, but when `live_module` is Some, every STORE_NAME/
    /// DELETE_NAME during this execution also mirrors into that module's
    /// own `dict` immediately — not just once execution finishes (see
    /// `Frame::live_module`'s doc comment for why this matters for
    /// circular imports).
    pub fn exec_code_with_module(
        &mut self,
        code: CodeObject,
        globals: Option<Rc<RefCell<HashMap<StrId, PyObjectRef>>>>,
        live_module: Option<PyObjectRef>,
    ) -> PyResult<PyObjectRef> {
        let g = globals.unwrap_or_else(|| self.globals.clone());
        let mut frame = self.acquire_frame(Rc::new(code), g, Rc::clone(&self.builtins), None);
        frame.live_module = live_module;
        self.push_frame(frame);
        let result = self.execute();
        if let Some(frame) = self.frames.pop() {
            self.release_frame(frame);
        }
        result
    }

    /// Populate the type registry with type objects for all builtin types.
    /// This is called during VM initialization so that builtin_type_of()
    /// can return real Type objects instead of string names.
    pub fn populate_type_registry(&mut self) {
        let type_names = [
            "NoneType",
            "bool",
            "int",
            "float",
            "str",
            "bytes",
            "bytearray",
            "list",
            "tuple",
            "dict",
            "set",
            "frozenset",
            "range",
            "slice",
            "function",
            "builtin_function_or_method",
            "builtin_method",
            "module",
            "type",
            "cell",
            "method",
            "partial",
            "property",
            "staticmethod",
            "classmethod",
            "generator",
            "coroutine",
            "Exception",
            "super",
            "lock",
            "RLock",
            "Event",
            "Queue",
            "Thread",
            "file",
            "socket",
            "capsule",
            "re.Pattern",
            "future_await_iterator",
            "enumerate",
            "list_iterator",
            "range_iterator",
        ];
        for name in &type_names {
            let type_obj = PyObjectRef::new(PyObject::Type {
                name: name.to_string(),
                dict: Box::new(TypeDict::default()),
                bases: vec![],
                mro: vec![],
            });
            self.type_registry.insert(name.to_string(), type_obj);
        }
    }
}
