// Extracted from pyobject.rs — PyFunction struct (see parent file header).
// Kept small to stay <1k.
use super::*;

pub struct PyFunction {
    pub code: Rc<CodeObject>,
    pub globals: Rc<RefCell<HashMap<StrId, PyObjectRef>>>,
    pub defaults: Vec<PyObjectRef>,
    pub closure: Vec<PyObjectRef>,
    pub dict: HashMap<String, PyObjectRef>,
    /// Live user globals dict for functions built via
    /// `types.FunctionType(code, dict)` (CPython: `__globals__` IS that
    /// dict — writes stay visible). Plain `def` functions leave it
    /// `None` (their globals are the shared module map). When set, the
    /// call path mirrors frame writes back after each call.
    pub globals_dict: Option<PyObjectRef>,
    pub jit_ptr: std::cell::Cell<usize>,
    pub jit_consts: std::cell::RefCell<Vec<PyObjectRef>>,
    /// Memoized `__name__` / `__qualname__` string objects. Attribute
    /// reads materialize these from the code object when never
    /// explicitly assigned — without memoization every read built a
    /// FRESH heap string (for names over the SmallStr inline limit),
    /// so `f.__qualname__ is f.__qualname__` was False (real trigger:
    /// test_decorators.py's check_wrapper_attrs assertIs). Explicit
    /// assignment still wins: reads check `dict` first, and clones
    /// start unmemoized (recomputed from their own code object).
    pub name_memo: std::cell::RefCell<Option<PyObjectRef>>,
    pub qualname_memo: std::cell::RefCell<Option<PyObjectRef>>,
}

impl Clone for PyFunction {
    fn clone(&self) -> Self {
        PyFunction {
            code: self.code.clone(),
            globals: self.globals.clone(),
            defaults: self.defaults.clone(),
            closure: self.closure.clone(),
            dict: self.dict.clone(),
            globals_dict: self.globals_dict.clone(),
            jit_ptr: std::cell::Cell::new(self.jit_ptr.get()),
            jit_consts: std::cell::RefCell::new(self.jit_consts.borrow().clone()),
            name_memo: std::cell::RefCell::new(None),
            qualname_memo: std::cell::RefCell::new(None),
        }
    }
}
