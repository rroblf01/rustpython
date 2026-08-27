use crate::bytecode::CodeObject;
use crate::interner::{self, InternedMap, StrId};
use crate::object::{PyError, PyObjectRef, PyResult};
use smallvec::SmallVec;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Clone)]
pub struct Frame {
    pub code: Rc<CodeObject>,
    pub locals: InternedMap<PyObjectRef>,
    pub fast_locals: Vec<Option<PyObjectRef>>,
    pub globals: Rc<RefCell<HashMap<StrId, PyObjectRef>>>,
    pub builtins: Rc<HashMap<StrId, PyObjectRef>>,
    pub stack: SmallVec<[PyObjectRef; 4]>,
    pub ip: usize,
    pub base_sp: usize,
    pub exception_handlers: Box<Vec<ExceptionHandler>>,
    pub closure: Box<Vec<PyObjectRef>>,
    /// Active exception for re-raise. Set by PUSH_EXC_INFO, consumed by RERAISE.
    /// This is separate from the value stack so that POP_EXCEPT (which pops the
    /// exception from the value stack) does not break RERAISE in try/finally blocks.
    pub active_exception: Option<Box<PyObjectRef>>,
    /// Previous `active_exception` values, one per nested PUSH_EXC_INFO.
    /// POP_EXCEPT restores the last one, so a bare `raise` after an inner
    /// handler finishes re-raises the OUTER handler's exception (CPython's
    /// exc_info stack semantics) instead of the stale inner one.
    pub active_exception_stack: Vec<Option<Box<PyObjectRef>>>,
    /// Cached Python `frame` object for this frame — created lazily and
    /// REUSED so that `sys._getframe()` and an exception traceback's
    /// `tb_frame` for the same live frame are the SAME object
    /// (`tb.tb_frame is sys._getframe()`, which CPython's own test_raise
    /// asserts). Invalidate when the frame is released/reacquired.
    pub frame_object: Option<PyObjectRef>,
    /// Cached f_locals dict handed out via the Python `frame` object; kept
    /// here so STORE_FAST/STORE_NAME/etc. can refresh its contents in place
    /// (the frame object's attribute stays the SAME PyObject across accesses
    /// — CPython identity requirement) while still reflecting new values.
    pub frame_locals_obj: Option<PyObjectRef>,
    /// Inline attribute cache — caches LOAD_ATTR results per instruction offset.
    /// Cleared when the frame is created; populated on first attribute access.
    pub attr_cache: Box<Vec<Option<(u64, PyObjectRef)>>>, // (type_version_tag, cached_value)
    /// Inline global cache — caches LOAD_GLOBAL results per instruction offset.
    pub global_cache: Box<Vec<Option<PyObjectRef>>>,
    /// Virtual registers for register-based bytecode execution.
    /// 256 virtual registers (u8 index) — no stack needed for most ops.
    pub registers: Box<Vec<Option<PyObjectRef>>>,
    /// Optional reference to the enclosing module's globals.
    /// Used by class bodies to resolve LOAD_NAME against module-level names
    /// and by MAKE_FUNCTION to set __module__ on created functions.
    pub module_globals: Option<Rc<RefCell<HashMap<StrId, PyObjectRef>>>>,
    /// First-insertion order of names STORE_NAME'd into this frame's
    /// `globals` — only populated for class-body frames (set up by
    /// `__build_class__`), since that's the one case where order is
    /// user-visible (class namespaces, and anything a metaclass inspects,
    /// e.g. enum member definition order). `globals` itself is a plain
    /// HashMap with no ordering guarantee; `None` for ordinary module/
    /// function frames, where nothing currently depends on order and
    /// tracking it would be pure overhead.
    pub name_order: Option<Rc<RefCell<Vec<String>>>>,
    /// The PyObject::Module this frame is the top-level execution of, if any.
    /// A module's own `dict` is otherwise only synced from `globals` once
    /// the whole body finishes executing (see `exec_module_source`) — so
    /// any attribute access on the module object *while it's still
    /// mid-execution* (e.g. a circular import reading a name defined
    /// earlier in the same file) would see a stale/empty dict. Real
    /// CPython avoids this because `module.__dict__` IS the executing
    /// frame's globals, not a separate copy. Mirroring every STORE_NAME
    /// into this live module's dict (see STORE_NAME/DELETE_NAME) gives the
    /// same effect generally, for every module, not just via IMPORT_FROM's
    /// narrower ancestor-frame fallback.
    pub live_module: Option<PyObjectRef>,
    /// While resuming a generator whose body contains `yield from`, the
    /// currently-active sub-iterator. `generator.throw()` delegates to this
    /// iterator's own `.throw()` instead of injecting the exception into the
    /// outer generator's frame (CPython semantics).
    pub yield_from_iter: Option<PyObjectRef>,
    /// Index of the previous (calling) frame in `vm.frames`, if any.
    /// Used to implement `frame.f_back` — the previous frame in the call stack.
    pub back: Option<usize>,
}

#[derive(Clone)]
pub struct ExceptionHandler {
    pub instr_addr: usize,
    pub stack_depth: usize,
}

impl Frame {
    pub fn new(
        code: Rc<CodeObject>,
        globals: Rc<RefCell<HashMap<StrId, PyObjectRef>>>,
        builtins: Rc<HashMap<StrId, PyObjectRef>>,
        module_globals: Option<Rc<RefCell<HashMap<StrId, PyObjectRef>>>>,
    ) -> Self {
        let instr_count = code.instructions.len();
        let names_len = code.names.len();
        Frame {
            fast_locals: vec![None; code.nlocals],
            code,
            locals: InternedMap::new(),
            globals,
            builtins,
            stack: SmallVec::new(),
            ip: 0,
            base_sp: 0,
            exception_handlers: Box::new(Vec::new()),
            closure: Box::new(Vec::new()),
            active_exception: None,
            active_exception_stack: Vec::new(),
            frame_object: None,
            frame_locals_obj: None,
            attr_cache: Box::new(vec![None; names_len]),
            global_cache: Box::new(vec![None; instr_count]),
            registers: Box::new(Vec::new()),
            module_globals,
            name_order: None,
            live_module: None,
            yield_from_iter: None,
            back: None,
        }
    }

    pub fn push(&mut self, obj: PyObjectRef) {
        self.stack.push(obj);
    }

    pub fn pop(&mut self) -> PyResult<PyObjectRef> {
        self.stack.pop().ok_or_else(|| {
            let instr_ip = if self.ip > 0 { self.ip - 1 } else { 0 };
            let op_str = if instr_ip < self.code.instructions.len() {
                format!("{:?}", self.code.instructions[instr_ip].op)
            } else {
                "END".to_string()
            };
            let arg = if instr_ip < self.code.instructions.len() {
                self.code.instructions[instr_ip].arg
            } else {
                0
            };
            let line_no = if instr_ip < self.code.instructions.len() {
                self.code.line_number(instr_ip)
            } else {
                0
            };
            PyError::runtime_error(format!(
                "stack underflow at {} arg={} line={} code={} file={}",
                op_str, arg, line_no, self.code.name, self.code.filename
            ))
        })
    }

    pub fn peek(&self, depth: usize) -> PyResult<PyObjectRef> {
        if depth >= self.stack.len() {
            let instr_ip = if self.ip > 0 { self.ip - 1 } else { 0 };
            let _op_str = if instr_ip < self.code.instructions.len() {
                format!("{:?}", self.code.instructions[instr_ip].op)
            } else {
                "END".to_string()
            };
            return Err(PyError::runtime_error("stack underflow (peek)"));
        }
        Ok(self.stack[self.stack.len() - 1 - depth].clone())
    }

    pub fn insert_local(&mut self, name: &str, val: PyObjectRef) -> Option<PyObjectRef> {
        self.locals.insert(interner::intern(name), val)
    }

    pub fn get_local(&self, name: &str) -> Option<&PyObjectRef> {
        self.locals.get(interner::intern(name))
    }

    pub fn remove_local(&mut self, name: &str) -> Option<PyObjectRef> {
        let sid = interner::intern(name);
        let mut out = self.locals.remove(sid);
        // `del x` in a function must ALSO clear the fast-local slot, or the
        // frame keeps the only meaningful reference alive forever (observed:
        // __del__ finalizers never fired because the retained slot kept
        // Rc::strong_count above the release threshold).
        if let Some(idx) = self.code.varnames.iter().position(|&n| n == sid) {
            if idx < self.fast_locals.len() {
                out = self.fast_locals[idx].take().or(out);
            }
        }
        out
    }

    pub fn contains_local(&self, name: &str) -> bool {
        self.locals.contains_key(interner::intern(name))
    }
}
