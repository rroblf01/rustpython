use crate::bytecode::CodeObject;
use crate::interner::StrId;
use crate::object::PyObjectRef;
use crate::vm::{Frame, VirtualMachine};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

impl VirtualMachine {
    pub(crate) fn acquire_frame(
        &mut self,
        code: Rc<CodeObject>,
        globals: Rc<RefCell<HashMap<StrId, PyObjectRef>>>,
        builtins: Rc<HashMap<StrId, PyObjectRef>>,
        module_globals: Option<Rc<RefCell<HashMap<StrId, PyObjectRef>>>>,
    ) -> Frame {
        if let Some(mut frame) = self.frame_pool.pop() {
            let nlocals = code.nlocals;
            let names_len = code.names.len();
            let instr_len = code.instructions.len();
            frame.code = code;
            frame.globals = globals;
            frame.builtins = builtins;
            frame.module_globals = module_globals;
            frame.fast_locals.clear();
            frame.fast_locals.resize(nlocals, None);
            frame.locals.clear();
            frame.stack.clear();
            frame.ip = 0;
            frame.base_sp = 0;
            frame.exception_handlers.clear();
            frame.closure.clear();
            frame.active_exception = None;
            frame.attr_cache.clear();
            frame.attr_cache.resize(names_len, None);
            frame.global_cache.clear();
            frame.global_cache.resize(instr_len, None);
            frame.registers.clear();
            frame.name_order = None;
            frame.live_module = None;
            frame.yield_from_iter = None;
            frame.frame_object = None;
            frame.frame_locals_obj = None;
            frame.back = None;
            frame
        } else {
            Frame::new(code, globals, builtins, module_globals)
        }
    }

    pub(crate) fn release_frame(&mut self, frame: Frame) {
        if self.frame_pool.len() < 32 {
            let mut frame = frame;
            // Drop every retained reference to Python objects BEFORE pooling.
            // Without this, pooled frames keep the last call's fast_locals /
            // stack / caches alive indefinitely, which (a) pins arbitrary
            // user objects as invisible GC roots (cycle_gc sees them as
            // eternal external referrers and can never collect any cycle
            // whose members passed through a function call), and (b) delays
            // memory reclamation until the slot happens to be reused.
            frame.fast_locals.clear();
            frame.locals.clear();
            frame.stack.clear();
            frame.closure.clear();
            frame.exception_handlers.clear();
            frame.active_exception = None;
            frame.attr_cache.clear();
            frame.global_cache.clear();
            frame.registers.clear();
            frame.name_order = None;
            frame.live_module = None;
            frame.yield_from_iter = None;
            frame.frame_object = None;
            frame.frame_locals_obj = None;
            frame.back = None;
            self.frame_pool.push(frame);
        }
    }

    /// Push a frame onto the VM's frame stack, setting its `back` field
    /// to point to the current frame (if any) for `frame.f_back` support.
    pub(crate) fn push_frame(&mut self, mut frame: Frame) {
        frame.back = if self.frames.is_empty() {
            None
        } else {
            Some(self.frames.len() - 1)
        };
        self.frames.push(frame);
    }
}
