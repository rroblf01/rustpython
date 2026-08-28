// Extracted from pyobject.rs — PyFunction struct (see parent file header).
// Kept small to stay <1k.
use super::*;

pub struct PyFunction {
    pub code: Rc<CodeObject>,
    pub globals: Rc<RefCell<HashMap<StrId, PyObjectRef>>>,
    pub defaults: Vec<PyObjectRef>,
    pub closure: Vec<PyObjectRef>,
    pub dict: HashMap<String, PyObjectRef>,
    pub jit_ptr: std::cell::Cell<usize>,
    pub jit_consts: std::cell::RefCell<Vec<PyObjectRef>>,
}

impl Clone for PyFunction {
    fn clone(&self) -> Self {
        PyFunction {
            code: self.code.clone(),
            globals: self.globals.clone(),
            defaults: self.defaults.clone(),
            closure: self.closure.clone(),
            dict: self.dict.clone(),
            jit_ptr: std::cell::Cell::new(self.jit_ptr.get()),
            jit_consts: std::cell::RefCell::new(self.jit_consts.borrow().clone()),
        }
    }
}
