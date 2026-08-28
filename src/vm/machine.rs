use crate::interner::StrId;
use crate::object::PyObjectRef;
use crate::vm::frame::Frame;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
#[cfg(feature = "jit")]
use crate::jit::JitCompiler;

pub struct VirtualMachine {
    pub frames: Vec<Frame>,
    pub builtins: Rc<HashMap<StrId, PyObjectRef>>,
    pub modules: HashMap<String, PyObjectRef>,
    pub globals: Rc<RefCell<HashMap<StrId, PyObjectRef>>>,
    #[cfg(feature = "jit")]
    pub jit: RefCell<JitCompiler>,
    /// Execution profile counters — how many times each instruction ran.
    /// Indexed by (function_id, instruction_offset). Used by JIT to
    /// identify hot paths for native compilation.
    pub profile: RefCell<HashMap<usize, Vec<u32>>>,
    pub frame_pool: Vec<Frame>,
    /// Line number of the last instruction executed. Used for error reporting.
    pub last_error_line: Option<usize>,
    /// Filename of the frame the last instruction executed in. Used for error reporting.
    pub last_error_file: Option<String>,
    /// Call-stack snapshot (filename, line, function name), outermost first,
    /// captured the moment an exception is found to have no handler anywhere
    /// on the current frame stack. Used for top-level traceback printing.
    pub last_traceback: Vec<(String, usize, String)>,
    /// Type registry: maps type names to PyObject::Type objects.
    /// Used by builtin_type_of() to return real type objects instead of strings.
    pub type_registry: HashMap<String, PyObjectRef>,
    /// Current exception info for sys.exc_info()
    pub exc_type: Option<PyObjectRef>,
    pub exc_value: Option<PyObjectRef>,
    pub exc_traceback: Option<PyObjectRef>,
    /// Stack of exceptions currently being handled (VM-global so it survives
    /// function calls made from inside an except handler). Pushed by
    /// PUSH_EXC_INFO, popped by POP_EXCEPT. A new `raise` inside a handler
    /// chains the innermost handled exception as `__context__` (PEP 3134
    /// implicit chaining); when empty but an exception is in flight
    /// (e.g. `finally:` re-raising over a propagating exception), the
    /// propagating exception chains instead.
    ///
    /// Each entry carries the value-stack depth at which the handled
    /// exception was pushed: when a new exception's unwind truncates the
    /// value stack below that depth (in `handle_exception`), the handler was
    /// abandoned mid-body (its `POP_EXCEPT` epilogue never ran) and the entry
    /// is dropped, so a later unrelated `raise` cannot be polluted by it.
    pub exc_context_stack: Vec<(PyObjectRef, usize)>,
    /// The most recently raised exception still propagating (cleared when it
    /// is caught by a handler — PUSH_EXC_INFO resets it — or replaced).
    pub propagating_exc: Option<PyObjectRef>,
    /// `sys.getrecursionlimit()`/`setrecursionlimit()` — consulted by
    /// `call_function`'s own `self.frames.len()` depth guard (see there for
    /// why this exists at all). Real trigger: CPython's own `test.support.
    /// infinite_recursion(N)` context manager temporarily lowers this to
    /// make deliberately-infinite-recursion tests fail fast instead of
    /// grinding through hundreds of real frames first.
    pub recursion_limit: usize,
}
