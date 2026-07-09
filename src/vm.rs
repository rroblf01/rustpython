use std::collections::HashMap;
use std::rc::Rc;
use std::cell::RefCell;
use num_bigint::BigInt;
use crate::bytecode::*;
use crate::object::*;
use crate::object::ObjectAccess;

#[derive(Clone)]
pub struct Frame {
    pub code: CodeObject,
    pub locals: HashMap<String, PyObjectRef>,
    pub globals: Rc<RefCell<HashMap<String, PyObjectRef>>>,
    pub builtins: HashMap<String, PyObjectRef>,
    pub stack: Vec<PyObjectRef>,
    pub ip: usize,
    pub base_sp: usize,
    pub exception_handlers: Vec<ExceptionHandler>,
    pub return_value: Option<PyResult<PyObjectRef>>,
}

#[derive(Clone)]
pub struct ExceptionHandler {
    pub instr_addr: usize,
    pub stack_depth: usize,
    pub handler_type: HandlerType,
}

#[derive(Clone)]
pub enum HandlerType {
    Except,
    Finally,
}

impl Frame {
    pub fn new(
        code: CodeObject,
        globals: Rc<RefCell<HashMap<String, PyObjectRef>>>,
        builtins: HashMap<String, PyObjectRef>,
    ) -> Self {
        Frame {
            code,
            locals: HashMap::new(),
            globals,
            builtins,
            stack: Vec::new(),
            ip: 0,
            base_sp: 0,
            exception_handlers: Vec::new(),
            return_value: None,
        }
    }

    pub fn push(&mut self, obj: PyObjectRef) {
        self.stack.push(obj);
    }

    pub fn pop(&mut self) -> PyResult<PyObjectRef> {
        self.stack.pop().ok_or_else(|| PyError::runtime_error("stack underflow"))
    }

    pub fn peek(&self, depth: usize) -> PyResult<PyObjectRef> {
        if depth >= self.stack.len() {
            return Err(PyError::runtime_error("stack underflow (peek)"));
        }
        Ok(self.stack[self.stack.len() - 1 - depth].clone())
    }

    pub fn stack_size(&self) -> usize {
        self.stack.len()
    }
}

pub struct VirtualMachine {
    pub frames: Vec<Frame>,
    pub builtins: HashMap<String, PyObjectRef>,
    pub modules: HashMap<String, PyObjectRef>,
    pub globals: Rc<RefCell<HashMap<String, PyObjectRef>>>,
}

impl VirtualMachine {
    pub fn new() -> Self {
        let builtins = create_builtins();
        let globals_map = HashMap::from([
            ("__name__".to_string(), py_str("__main__")),
            ("__builtins__".to_string(), create_module("builtins", builtins.clone())),
        ]);
        let globals = Rc::new(RefCell::new(globals_map));
 
         let mut modules = HashMap::new();
         modules.insert("builtins".to_string(), create_module("builtins", builtins.clone()));
 
         VirtualMachine {
             frames: Vec::new(),
             builtins,
             modules,
             globals,
         }
    }

    pub fn run(&mut self, code: CodeObject) -> PyResult<PyObjectRef> {
        let frame = Frame::new(
            code,
            self.globals.clone(),
            self.builtins.clone(),
        );
        self.frames.push(frame);
        let result = self.execute();
        self.frames.pop();
        result
    }

    pub fn exec_code(&mut self, code: CodeObject, globals: Option<Rc<RefCell<HashMap<String, PyObjectRef>>>>) -> PyResult<PyObjectRef> {
        let g = globals.unwrap_or_else(|| self.globals.clone());
        let frame = Frame::new(code, g, self.builtins.clone());
        self.frames.push(frame);
        let result = self.execute();
        self.frames.pop();
        result
    }

    fn execute(&mut self) -> PyResult<PyObjectRef> {
        loop {
            let result = self.execute_instruction();
            match result {
                Ok(None) => continue,
                Ok(Some(val)) => return Ok(val),
                Err(e) => {
                    if matches!(&e, PyError::SystemExit(_)) {
                        return Err(e);
                    }
                    if matches!(&e, PyError::StopIteration) {
                        return Err(e);
                    }
                    if !self.handle_exception(&e) {
                        return Err(e);
                    }
                }
            }
        }
    }

    fn execute_instruction(&mut self) -> PyResult<Option<PyObjectRef>> {
        let (instr, _) = {
            let f = &self.frames[self.frames.len() - 1];
            let ip = f.ip;
            if ip >= f.code.instructions.len() {
                return Err(PyError::runtime_error("execution reached end of code"));
            }
            (f.code.instructions[ip].clone(), ip)
        };
        self.frames.last_mut().unwrap().ip += 1;

        match instr.op {
            Opcode::NOP => {}

            Opcode::LOAD_CONST => {
                let const_idx = instr.arg as usize;
                let const_val = self.frames.last().unwrap().code.consts.get(const_idx).ok_or_else(|| {
                    PyError::runtime_error(format!("constant index out of range: {}", const_idx))
                })?.clone();
                let obj = match const_val {
                    ConstValue::None => py_none(),
                    ConstValue::Bool(b) => py_bool(b),
                    ConstValue::Int(s) => {
                        let n: BigInt = s.parse().map_err(|_| {
                            PyError::value_error(format!("invalid integer: {}", s))
                        })?;
                        PyObjectRef::new(PyObject::Int(n))
                    }
                    ConstValue::Float(s) => {
                        let f: f64 = s.parse().map_err(|_| {
                            PyError::value_error(format!("invalid float: {}", s))
                        })?;
                        py_float(f)
                    }
                    ConstValue::String(s) => py_str(&s),
                    ConstValue::Code(code) => {
                        PyObjectRef::new(PyObject::Code(code))
                    }
                };
                self.frames.last_mut().unwrap().push(obj);
            }

            Opcode::LOAD_NAME => {
                let name_idx = instr.arg as usize;
                let name = self.frames.last().unwrap().code.names.get(name_idx).ok_or_else(|| {
                    PyError::runtime_error("name index out of range")
                })?.clone();
                let val = {
                    let f = &self.frames[self.frames.len() - 1];
                    f.locals.get(&name).cloned()
                        .or_else(|| f.globals.borrow().get(&name).cloned())
                        .or_else(|| f.builtins.get(&name).cloned())
                };
                match val {
                    Some(v) => self.frames.last_mut().unwrap().push(v),
                    None => return Err(PyError::name_error(format!("name '{}' is not defined", name))),
                }
            }

            Opcode::STORE_NAME => {
                let name_idx = instr.arg as usize;
                let name = self.frames.last().unwrap().code.names.get(name_idx).ok_or_else(|| {
                    PyError::runtime_error("name index out of range")
                })?.clone();
                let val = self.frames.last_mut().unwrap().pop()?;
                self.frames.last_mut().unwrap().globals.borrow_mut().insert(name, val);
            }

            Opcode::LOAD_FAST => {
                let var_idx = instr.arg as usize;
                let name = self.frames.last().unwrap().code.varnames.get(var_idx).ok_or_else(|| {
                    PyError::runtime_error("varname index out of range")
                })?.clone();
                let val = {
                    let f = &self.frames[self.frames.len() - 1];
                    f.locals.get(&name).cloned().or_else(|| {
                        for (k, v) in &f.locals {
                            if k == &name { return Some(v.clone()); }
                        }
                        None
                    })
                };
                match val {
                    Some(v) => self.frames.last_mut().unwrap().push(v),
                    None => return Err(PyError::name_error(format!("local variable '{}' referenced before assignment", name))),
                }
            }

            Opcode::STORE_FAST => {
                let var_idx = instr.arg as usize;
                let name = self.frames.last().unwrap().code.varnames.get(var_idx).ok_or_else(|| {
                    PyError::runtime_error("varname index out of range")
                })?.clone();
                let val = self.frames.last_mut().unwrap().pop()?;
                self.frames.last_mut().unwrap().locals.insert(name, val);
            }

            Opcode::LOAD_GLOBAL => {
                let name_idx = instr.arg as usize;
                let name = self.frames.last().unwrap().code.names.get(name_idx).ok_or_else(|| {
                    PyError::runtime_error("name index out of range")
                })?.clone();
                let val = {
                    let f = &self.frames[self.frames.len() - 1];
                    f.globals.borrow().get(&name).cloned()
                        .or_else(|| f.builtins.get(&name).cloned())
                };
                match val {
                    Some(v) => self.frames.last_mut().unwrap().push(v),
                    None => return Err(PyError::name_error(format!("name '{}' is not defined", name))),
                }
            }

            Opcode::STORE_GLOBAL => {
                let name_idx = instr.arg as usize;
                let name = self.frames.last().unwrap().code.names.get(name_idx).ok_or_else(|| {
                    PyError::runtime_error("name index out of range")
                })?.clone();
                let val = self.frames.last_mut().unwrap().pop()?;
                self.frames.last_mut().unwrap().globals.borrow_mut().insert(name, val);
            }

            Opcode::LOAD_DEREF => {
                let var_idx = instr.arg as usize;
                let (name, val) = {
                    let f = &self.frames[self.frames.len() - 1];
                    let name = if var_idx < f.code.cellvars.len() {
                        f.code.cellvars[var_idx].clone()
                    } else {
                        let fv_idx = var_idx - f.code.cellvars.len();
                        f.code.freevars.get(fv_idx).ok_or_else(|| {
                            PyError::runtime_error("freevar index out of range")
                        })?.clone()
                    };
                    let val = f.locals.get(&name).cloned();
                    (name, val)
                };
                match val {
                    Some(v) => {
                        let push_val = {
                            let obj = v.borrow();
                            match &*obj {
                                PyObject::Cell { value: Some(inner) } => inner.clone(),
                                PyObject::Cell { value: None } => return Err(PyError::name_error(format!("variable '{}' referenced before assignment", name))),
                                _ => v.clone(),
                            }
                        };
                        self.frames.last_mut().unwrap().push(push_val);
                    }
                    None => return Err(PyError::name_error(format!("variable '{}' not found", name))),
                }
            }

            Opcode::STORE_DEREF => {
                let var_idx = instr.arg as usize;
                let name = {
                    let f = &self.frames[self.frames.len() - 1];
                    if var_idx < f.code.cellvars.len() {
                        f.code.cellvars[var_idx].clone()
                    } else {
                        f.code.freevars[var_idx - f.code.cellvars.len()].clone()
                    }
                };
                let val = self.frames.last_mut().unwrap().pop()?;
                let cell = {
                    let f = &self.frames[self.frames.len() - 1];
                    f.locals.get(&name).cloned()
                };
                if let Some(cell) = cell {
                    let mut cell = cell.borrow_mut();
                    if let PyObject::Cell { value } = &mut *cell {
                        *value = Some(val);
                    }
                } else {
                    let cell_obj = PyObjectRef::new(PyObject::Cell { value: Some(val) });
                    self.frames.last_mut().unwrap().locals.insert(name, cell_obj);
                }
            }

            Opcode::DELETE_FAST => {
                let var_idx = instr.arg as usize;
                let name = self.frames.last().unwrap().code.varnames[var_idx].clone();
                self.frames.last_mut().unwrap().locals.remove(&name);
            }

            Opcode::DELETE_NAME => {
                let name_idx = instr.arg as usize;
                let name = self.frames.last().unwrap().code.names[name_idx].clone();
                self.frames.last_mut().unwrap().globals.borrow_mut().remove(&name);
            }

            Opcode::POP_TOP => {
                self.frames.last_mut().unwrap().pop()?;
            }

            Opcode::DUP_TOP => {
                let val = self.frames.last().unwrap().peek(0)?;
                self.frames.last_mut().unwrap().push(val);
            }

            Opcode::COPY => {
                let depth = instr.arg as usize;
                let val = self.frames.last().unwrap().peek(depth)?;
                self.frames.last_mut().unwrap().push(val);
            }

            Opcode::SWAP => {
                let i = instr.arg as usize;
                let len = self.frames.last().unwrap().stack.len();
                if i > 0 && i < len {
                    self.frames.last_mut().unwrap().stack.swap(len - 1, len - 1 - i);
                }
            }

            Opcode::RETURN_VALUE => {
                let val = self.frames.last_mut().unwrap().pop()?;
                return Ok(Some(val));
            }

            Opcode::PUSH_NULL => {
                self.frames.last_mut().unwrap().push(py_none());
            }

            Opcode::CALL => {
                let nargs = instr.arg as usize;
                let mut args = Vec::with_capacity(nargs);
                for _ in 0..nargs {
                    args.push(self.frames.last_mut().unwrap().pop()?);
                }
                let callable = self.frames.last_mut().unwrap().pop()?;
                let result = self.call_function(callable, args)?;
                self.frames.last_mut().unwrap().push(result);
            }

            Opcode::MAKE_FUNCTION => {
                let code_obj = self.frames.last_mut().unwrap().pop()?;
                let code = match &*code_obj.borrow() {
                    PyObject::Code(c) => c.as_ref().clone(),
                    _ => return Err(PyError::runtime_error("MAKE_FUNCTION: expected code object")),
                };
                let globals = self.frames.last().unwrap().globals.clone();
                let func = PyObjectRef::new(PyObject::Function {
                    code,
                    globals,
                    name: "<function>".to_string(),
                    defaults: Vec::new(),
                    closure: Vec::new(),
                });
                self.frames.last_mut().unwrap().push(func);
            }

            Opcode::BUILD_LIST => {
                let count = instr.arg as usize;
                let mut items = Vec::with_capacity(count);
                for _ in 0..count {
                    items.push(self.frames.last_mut().unwrap().pop()?);
                }
                items.reverse();
                self.frames.last_mut().unwrap().push(py_list(items));
            }

            Opcode::BUILD_TUPLE => {
                let count = instr.arg as usize;
                let mut items = Vec::with_capacity(count);
                for _ in 0..count {
                    items.push(self.frames.last_mut().unwrap().pop()?);
                }
                items.reverse();
                self.frames.last_mut().unwrap().push(py_tuple(items));
            }

            Opcode::BUILD_MAP => {
                self.frames.last_mut().unwrap().push(py_dict());
            }

            Opcode::BUILD_SET => {
                let count = instr.arg as usize;
                let mut items = Vec::with_capacity(count);
                for _ in 0..count {
                    items.push(self.frames.last_mut().unwrap().pop()?);
                }
                items.reverse();
                self.frames.last_mut().unwrap().push(PyObjectRef::new(PyObject::Set(items)));
            }

            Opcode::BUILD_STRING => {
                let count = instr.arg as usize;
                let mut parts = Vec::with_capacity(count);
                for _ in 0..count {
                    parts.push(self.frames.last_mut().unwrap().pop()?.str());
                }
                parts.reverse();
                self.frames.last_mut().unwrap().push(py_str(&parts.join("")));
            }

            Opcode::BUILD_SLICE => {
                let nargs = instr.arg as usize;
                let stop = if nargs >= 2 { Some(self.frames.last_mut().unwrap().pop()?) } else { None };
                let start = if nargs >= 1 { Some(self.frames.last_mut().unwrap().pop()?) } else { None };
                let step = if nargs >= 3 { Some(self.frames.last_mut().unwrap().pop()?) } else { None };
                self.frames.last_mut().unwrap().push(PyObjectRef::new(PyObject::Slice {
                    start: start.unwrap_or(py_none()),
                    stop: stop.unwrap_or(py_none()),
                    step: step.unwrap_or(py_none()),
                }));
            }

            Opcode::BINARY_OP => {
                let op = instr.arg;
                let right = self.frames.last_mut().unwrap().pop()?;
                let left = self.frames.last_mut().unwrap().pop()?;
                let result = match op {
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
                    _ => return Err(PyError::runtime_error(format!("unknown binary op: {}", op))),
                }?;
                self.frames.last_mut().unwrap().push(result);
            }

            Opcode::COMPARE_OP => {
                let op = instr.arg;
                let right = self.frames.last_mut().unwrap().pop()?;
                let left = self.frames.last_mut().unwrap().pop()?;
                let result = py_compare(&left, &right, op)?;
                self.frames.last_mut().unwrap().push(result);
            }

            Opcode::IS_OP => {
                let invert = instr.arg != 0;
                let right = self.frames.last_mut().unwrap().pop()?;
                let left = self.frames.last_mut().unwrap().pop()?;
                let is_same = left.is(&right);
                let result = if invert { !is_same } else { is_same };
                self.frames.last_mut().unwrap().push(py_bool(result));
            }

            Opcode::CONTAINS_OP => {
                let invert = instr.arg != 0;
                let right = self.frames.last_mut().unwrap().pop()?;
                let left = self.frames.last_mut().unwrap().pop()?;
                let result = contains_op(&right, &left)?;
                let result = if invert { !result } else { result };
                self.frames.last_mut().unwrap().push(py_bool(result));
            }

            Opcode::UNARY_NEGATIVE => {
                let val = self.frames.last_mut().unwrap().pop()?;
                let result = py_sub(&py_int(0), &val)?;
                self.frames.last_mut().unwrap().push(result);
            }

            Opcode::UNARY_NOT => {
                let val = self.frames.last_mut().unwrap().pop()?;
                self.frames.last_mut().unwrap().push(py_bool(!val.truthy()));
            }

            Opcode::UNARY_INVERT => {
                let val = self.frames.last_mut().unwrap().pop()?;
                let result = {
                    let obj = val.borrow();
                    match &*obj {
                        PyObject::Int(i) => py_int(!i),
                        _ => return Err(PyError::type_error("bad operand type for unary ~")),
                    }
                };
                self.frames.last_mut().unwrap().push(result);
            }

            Opcode::JUMP_FORWARD | Opcode::JUMP | Opcode::JUMP_BACKWARD => {
                let offset = instr.arg as usize;
                match instr.op {
                    Opcode::JUMP_FORWARD => {
                        self.frames.last_mut().unwrap().ip += offset;
                    }
                    Opcode::JUMP => {
                        self.frames.last_mut().unwrap().ip = offset;
                    }
                    Opcode::JUMP_BACKWARD => {
                        let cur_ip = self.frames.last().unwrap().ip;
                        self.frames.last_mut().unwrap().ip = cur_ip.wrapping_sub(offset).wrapping_sub(1);
                    }
                    _ => unreachable!(),
                }
            }

            Opcode::POP_JUMP_IF_FALSE => {
                let val = self.frames.last_mut().unwrap().pop()?;
                if !val.truthy() {
                    self.frames.last_mut().unwrap().ip = instr.arg as usize;
                }
            }

            Opcode::POP_JUMP_IF_TRUE => {
                let val = self.frames.last_mut().unwrap().pop()?;
                if val.truthy() {
                    self.frames.last_mut().unwrap().ip = instr.arg as usize;
                }
            }

            Opcode::POP_JUMP_IF_NONE => {
                let val = self.frames.last_mut().unwrap().pop()?;
                let is_none = {
                    matches!(&*val.borrow(), PyObject::None)
                };
                if is_none {
                    self.frames.last_mut().unwrap().ip = instr.arg as usize;
                }
            }

            Opcode::POP_JUMP_IF_NOT_NONE => {
                let val = self.frames.last_mut().unwrap().pop()?;
                let is_not_none = {
                    !matches!(&*val.borrow(), PyObject::None)
                };
                if is_not_none {
                    self.frames.last_mut().unwrap().ip = instr.arg as usize;
                }
            }

            Opcode::GET_ITER => {
                let val = self.frames.last_mut().unwrap().pop()?;
                let obj = val.borrow();
                match &*obj {
                    PyObject::List(v) => {
                        let iter = PyObjectRef::new(PyObject::List(v.clone()));
                        self.frames.last_mut().unwrap().push(iter);
                    }
                    PyObject::Tuple(v) => {
                        let iter = PyObjectRef::new(PyObject::List(v.clone()));
                        self.frames.last_mut().unwrap().push(iter);
                    }
                    PyObject::Str(s) => {
                        let chars: Vec<PyObjectRef> = s.chars().map(|c| py_str(&c.to_string())).collect();
                        self.frames.last_mut().unwrap().push(py_list(chars));
                    }
                    PyObject::Set(s) => {
                        self.frames.last_mut().unwrap().push(py_list(s.clone()));
                    }
                    _ => return Err(PyError::type_error(format!("'{}' object is not iterable", obj.type_name()))),
                }
            }

            Opcode::FOR_ITER => {
                let iter_val = self.frames.last().unwrap().peek(0)?;
                let is_empty = {
                    let obj = iter_val.borrow();
                    match &*obj {
                        PyObject::List(v) => v.is_empty(),
                        _ => return Err(PyError::type_error("for_iter on non-iterable")),
                    }
                };
                if is_empty {
                    self.frames.last_mut().unwrap().ip = instr.arg as usize;
                } else {
                    let val = self.frames.last_mut().unwrap().pop()?;
                    let item = {
                        let mut obj = val.borrow_mut();
                        if let PyObject::List(list) = &mut *obj {
                            list.remove(0)
                        } else {
                            unreachable!()
                        }
                    };
                    self.frames.last_mut().unwrap().push(val);
                    self.frames.last_mut().unwrap().push(item);
                }
            }

            Opcode::LOAD_ATTR => {
                let name_idx = instr.arg as usize;
                let name = self.frames.last().unwrap().code.names.get(name_idx).ok_or_else(|| {
                    PyError::runtime_error("name index out of range")
                })?.clone();
                let obj = self.frames.last_mut().unwrap().pop()?;
                let result = obj.borrow().get_attribute(&name).or_else(|_| {
                    Err(PyError::attribute_error(format!("'{}' object has no attribute '{}'",
                        obj.borrow().type_name(), name)))
                })?;
                self.frames.last_mut().unwrap().push(result);
            }

            Opcode::STORE_ATTR => {
                let name_idx = instr.arg as usize;
                let name = self.frames.last().unwrap().code.names.get(name_idx).ok_or_else(|| {
                    PyError::runtime_error("name index out of range")
                })?.clone();
                let val = self.frames.last_mut().unwrap().pop()?;
                let obj = self.frames.last_mut().unwrap().pop()?;
                obj.borrow_mut().set_attribute(&name, val)?;
            }

            Opcode::STORE_SUBSCR => {
                let val = self.frames.last_mut().unwrap().pop()?;
                let index = self.frames.last_mut().unwrap().pop()?;
                let obj = self.frames.last_mut().unwrap().pop()?;
                py_setitem(&obj, &index, val)?;
            }

            Opcode::LIST_APPEND => {
                let val = self.frames.last_mut().unwrap().pop()?;
                let list = self.frames.last().unwrap().peek(instr.arg as usize)?;
                let mut obj = list.borrow_mut();
                if let PyObject::List(v) = &mut *obj {
                    v.push(val);
                } else {
                    return Err(PyError::runtime_error("LIST_APPEND on non-list"));
                }
            }

            Opcode::SET_ADD => {
                let val = self.frames.last_mut().unwrap().pop()?;
                let set = self.frames.last().unwrap().peek(instr.arg as usize)?;
                let mut obj = set.borrow_mut();
                if let PyObject::Set(v) = &mut *obj {
                    v.push(val);
                } else {
                    return Err(PyError::runtime_error("SET_ADD on non-set"));
                }
            }

            Opcode::MAP_ADD => {
                let val = self.frames.last_mut().unwrap().pop()?;
                let key = self.frames.last_mut().unwrap().pop()?;
                let map = self.frames.last().unwrap().peek(0)?;
                let mut obj = map.borrow_mut();
                if let PyObject::Dict(d) = &mut *obj {
                    let key_str = key.str();
                    d.insert(key_str, val);
                } else {
                    return Err(PyError::runtime_error("MAP_ADD on non-dict"));
                }
            }

            Opcode::UNPACK_SEQUENCE => {
                let count = instr.arg as usize;
                let seq = self.frames.last_mut().unwrap().pop()?;
                let items = {
                    let obj = seq.borrow();
                    match &*obj {
                        PyObject::List(v) | PyObject::Tuple(v) => {
                            if v.len() != count {
                                return Err(PyError::value_error(format!(
                                    "cannot unpack {} items into {} values", v.len(), count
                                )));
                            }
                            v.clone()
                        }
                        _ => return Err(PyError::type_error("cannot unpack non-iterable")),
                    }
                };
                for item in items.into_iter().rev() {
                    self.frames.last_mut().unwrap().push(item);
                }
            }

            Opcode::SETUP_FINALLY => {
                let stack_depth = self.frames.last().unwrap().stack.len();
                let handler = ExceptionHandler {
                    instr_addr: instr.arg as usize,
                    stack_depth,
                    handler_type: HandlerType::Except,
                };
                self.frames.last_mut().unwrap().exception_handlers.push(handler);
            }

            Opcode::SETUP_CLEANUP => {
                let stack_depth = self.frames.last().unwrap().stack.len();
                let handler = ExceptionHandler {
                    instr_addr: instr.arg as usize,
                    stack_depth,
                    handler_type: HandlerType::Finally,
                };
                self.frames.last_mut().unwrap().exception_handlers.push(handler);
            }

            Opcode::POP_BLOCK => {
                self.frames.last_mut().unwrap().exception_handlers.pop();
            }

            Opcode::PUSH_EXC_INFO => {}

            Opcode::POP_EXCEPT => {}

            Opcode::CHECK_EXC_MATCH => {
                let exc = self.frames.last_mut().unwrap().pop()?;
                let expected = self.frames.last_mut().unwrap().pop()?;
                let matched = match &*exc.borrow() {
                    PyObject::Exception { typ, .. } => {
                        match &*expected.borrow() {
                            PyObject::Str(s) => typ == s,
                            _ => false,
                        }
                    }
                    _ => false,
                };
                self.frames.last_mut().unwrap().push(py_bool(matched));
            }

            Opcode::RERAISE => {
                return Err(PyError::runtime_error("re-raise"));
            }

            Opcode::RAISE_VARARGS => {
                let nargs = instr.arg;
                match nargs {
                    0 => return Err(PyError::runtime_error("re-raise")),
                    1 => {
                        let exc = self.frames.last_mut().unwrap().pop()?;
                        let msg = match &*exc.borrow() {
                            PyObject::Str(s) => s.clone(),
                            PyObject::Exception { args, .. } => {
                                if !args.is_empty() { args[0].str() } else { "".to_string() }
                            }
                            _ => return Err(PyError::type_error("exceptions must be str or Exception instances")),
                        };
                        return Err(PyError::Exception(msg, exc));
                    }
                    2 => {
                        let cause = self.frames.last_mut().unwrap().pop()?;
                        let exc = self.frames.last_mut().unwrap().pop()?;
                        return Err(PyError::Exception(format!("{} (caused by {})", exc.str(), cause.str()), exc));
                    }
                    _ => return Err(PyError::runtime_error("invalid RAISE_VARARGS count")),
                }
            }

            Opcode::IMPORT_NAME => {
                let name_idx = instr.arg as usize;
                let name = self.frames.last().unwrap().code.names.get(name_idx).ok_or_else(|| {
                    PyError::runtime_error("name index out of range")
                })?.clone();
                self.frames.last_mut().unwrap().pop()?;
                self.frames.last_mut().unwrap().pop()?;
                if self.modules.contains_key(&name) {
                    self.frames.last_mut().unwrap().push(self.modules[&name].clone());
                } else {
                    let module = create_module(&name, HashMap::new());
                    self.modules.insert(name.clone(), module.clone());
                    self.frames.last_mut().unwrap().push(module);
                }
            }

            Opcode::IMPORT_FROM => {
                let name_idx = instr.arg as usize;
                let name = self.frames.last().unwrap().code.names.get(name_idx).ok_or_else(|| {
                    PyError::runtime_error("name index out of range")
                })?.clone();
                let module = self.frames.last().unwrap().peek(0)?;
                let obj = module.borrow();
                match &*obj {
                    PyObject::Module { dict, .. } => {
                        if let Some(val) = dict.get(&name) {
                            self.frames.last_mut().unwrap().push(val.clone());
                        } else {
                            self.frames.last_mut().unwrap().push(py_none());
                        }
                    }
                    _ => return Err(PyError::runtime_error("IMPORT_FROM on non-module")),
                }
            }

            Opcode::LOAD_BUILD_CLASS => {
                let builtin_type = self.builtins.get("type").cloned().unwrap_or_else(|| {
                    PyObjectRef::new(PyObject::BuiltinFunction {
                        name: "type".to_string(),
                        func: builtin_type_of,
                    })
                });
                self.frames.last_mut().unwrap().push(builtin_type);
            }

            Opcode::LOAD_CLOSURE => {
                let idx = instr.arg as usize;
                let (name, cell) = {
                    let f = &self.frames[self.frames.len() - 1];
                    let name = if idx < f.code.cellvars.len() {
                        f.code.cellvars[idx].clone()
                    } else {
                        f.code.freevars[idx - f.code.cellvars.len()].clone()
                    };
                    let cell = f.locals.get(&name).cloned().unwrap_or_else(|| {
                        PyObjectRef::new(PyObject::Cell { value: None })
                    });
                    (name, cell)
                };
                self.frames.last_mut().unwrap().push(cell);
            }

            Opcode::FORMAT_SIMPLE => {
                let val = self.frames.last_mut().unwrap().pop()?;
                self.frames.last_mut().unwrap().push(py_str(&val.str()));
            }

            Opcode::FORMAT_WITH_SPEC => {
                let spec = self.frames.last_mut().unwrap().pop()?;
                let val = self.frames.last_mut().unwrap().pop()?;
                self.frames.last_mut().unwrap().push(py_str(&val.str()));
            }

            Opcode::CONVERT_VALUE => {
                let conversion = instr.arg;
                let val = self.frames.last_mut().unwrap().pop()?;
                let result = match conversion {
                    0 => py_str(&val.str()),
                    1 => py_str(&val.repr()),
                    2 => py_str(&val.str()),
                    _ => return Err(PyError::runtime_error("unknown conversion type")),
                };
                self.frames.last_mut().unwrap().push(result);
            }

            Opcode::LOAD_LOCALS => {
                self.frames.last_mut().unwrap().push(py_dict());
            }

            Opcode::SETUP_ANNOTATIONS => {}

            Opcode::POP_ITER => {
                self.frames.last_mut().unwrap().pop()?;
            }

            Opcode::SETUP_WITH => {
                // Simplified: just enter the context manager
                let mgr = self.frames.last().unwrap().peek(0)?;
                let exit_method = {
                    let obj = mgr.borrow();
                    obj.get_attribute("__exit__").ok()
                };
                let enter_method = {
                    let obj = mgr.borrow();
                    obj.get_attribute("__enter__").ok()
                };
                if let Some(enter) = enter_method {
                    let result = self.call_function(enter, vec![])?;
                    self.frames.last_mut().unwrap().push(result);
                } else {
                    // Enter and push None as result (simplified)
                    self.frames.last_mut().unwrap().push(py_none());
                }
            }

            Opcode::YIELD_VALUE => {
                let val = self.frames.last_mut().unwrap().pop()?;
                return Ok(Some(val));
            }

            Opcode::RETURN_GENERATOR => {
                self.frames.last_mut().unwrap().push(py_none());
            }

            _ => return Err(PyError::runtime_error(format!("unimplemented opcode: {:?}", instr.op))),
        }
        Ok(None)
    }

    fn call_function(&mut self, callable: PyObjectRef, args: Vec<PyObjectRef>) -> PyResult<PyObjectRef> {
        let type_name = callable.borrow().type_name();

        if let PyObject::BuiltinFunction { func, .. } = &*callable.borrow() {
            let func = *func;
            return func(&args);
        }

        if let PyObject::BuiltinMethod { func, self_obj, .. } = &*callable.borrow() {
            let func = *func;
            let self_obj = self_obj.clone();
            let mut new_args = vec![self_obj];
            new_args.extend(args);
            return func(&new_args);
        }

        if let PyObject::Function { code, globals: func_globals, .. } = &*callable.borrow() {
            let code = code.clone();
            let func_globals = func_globals.clone();
            let mut new_frame = Frame::new(code, func_globals, self.builtins.clone());
            for (i, arg_name) in new_frame.code.varnames.iter().enumerate() {
                if i < args.len() {
                    new_frame.locals.insert(arg_name.clone(), args[i].clone());
                } else {
                    new_frame.locals.insert(arg_name.clone(), py_none());
                }
            }
            self.frames.push(new_frame);
            let result = self.execute();
            self.frames.pop();
            return result;
        }

        if let PyObject::Type { dict, .. } = &*callable.borrow() {
            let instance = PyObjectRef::new(PyObject::Instance {
                typ: callable.clone(),
                dict: HashMap::new(),
            });
            if let Some(init_func) = dict.get("__init__") {
                if let PyObject::BuiltinFunction { func, .. } = &*init_func.borrow() {
                    let func = *func;
                    let mut init_args = vec![instance.clone()];
                    init_args.extend(args);
                    func(&init_args)?;
                }
            }
            return Ok(instance);
        }

        Err(PyError::type_error(format!("'{}' object is not callable", type_name)))
    }

    fn handle_exception(&mut self, error: &PyError) -> bool {
        for frame in self.frames.iter_mut().rev() {
            while let Some(handler) = frame.exception_handlers.pop() {
                frame.stack.truncate(handler.stack_depth);
                frame.ip = handler.instr_addr;
                let exc_obj = PyObjectRef::new(PyObject::Exception {
                    typ: match error {
                        PyError::TypeError(_) => "TypeError",
                        PyError::ValueError(_) => "ValueError",
                        PyError::NameError(_) => "NameError",
                        PyError::AttributeError(_) => "AttributeError",
                        PyError::IndexError(_) => "IndexError",
                        PyError::KeyError(_) => "KeyError",
                        PyError::ZeroDivisionError(_) => "ZeroDivisionError",
                        PyError::RuntimeError(_) => "RuntimeError",
                        PyError::StopIteration => "StopIteration",
                        PyError::AssertionError(_) => "AssertionError",
                        PyError::ImportError(_) => "ImportError",
                        _ => "Exception",
                    }.to_string(),
                    args: vec![py_str(&error.message())],
                    cause: None,
                });
                frame.push(exc_obj);
                return true;
            }
        }
        false
    }
}
