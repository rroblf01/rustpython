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
        let mut builtins = create_builtins();
        let globals_map = HashMap::from([
            ("__name__".to_string(), py_str("__main__")),
            ("__builtins__".to_string(), create_module("builtins", builtins.clone())),
        ]);
        let globals = Rc::new(RefCell::new(globals_map));
 
         let mut modules = HashMap::new();
         modules.insert("builtins".to_string(), create_module("builtins", builtins.clone()));
         modules.insert("math".to_string(), create_module("math", create_math_dict()));

         let sys_dict = create_sys_dict();
         modules.insert("sys".to_string(), create_module("sys", sys_dict.clone()));
         builtins.extend(sys_dict);

         let os_dict = create_os_dict();
         modules.insert("os".to_string(), create_module("os", os_dict.clone()));
 
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

    fn import_module_from_file(&self, name: &str) -> Result<PyObjectRef, String> {
        // Try to find and load a .py file from common paths
        let paths = vec![
            format!("./{}.py", name),
            format!("./Lib/{}.py", name),
            format!("/usr/lib/python3/{}.py", name),
        ];
        for path in &paths {
            if let Ok(source) = std::fs::read_to_string(path) {
                let mut parser = crate::parser::Parser::new(&source);
                let program = parser.parse_program().map_err(|e| format!("Parse error in {}: {}", name, e))?;
                let mut compiler = crate::compiler::Compiler::new();
                let code = compiler.compile(&program, path).map_err(|e| format!("Compile error in {}: {}", name, e))?;
                let mut vm = super::VirtualMachine::new();
                let globals = std::rc::Rc::new(std::cell::RefCell::new(std::collections::HashMap::new()));
                let result = vm.exec_code(code, Some(globals.clone()));
                match result {
                    Ok(_) => {
                        let module = create_module(name, globals.borrow().clone());
                        return Ok(module);
                    }
                    Err(e) => return Err(format!("Error importing {}: {}", name, e)),
                }
            }
        }
        Err(format!("Module '{}' not found", name))
    }

    pub fn execute(&mut self) -> PyResult<PyObjectRef> {
        loop {
            let result = self.execute_instruction();
            match result {
                Ok(None) => continue,
                Ok(Some(val)) => return Ok(val),
                Err(e) => {
                    if matches!(&e, PyError::SystemExit(_)) {
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
                args.reverse();
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
                    dict: HashMap::new(),
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
                    PyObject::Generator { .. } => {
                        drop(obj);
                        self.frames.last_mut().unwrap().push(val);
                    }
                    _ => return Err(PyError::type_error(format!("'{}' object is not iterable", obj.type_name()))),
                }
            }

            Opcode::FOR_ITER => {
                let iter_val = self.frames.last().unwrap().peek(0)?;
                let is_generator = matches!(&*iter_val.borrow(), PyObject::Generator { .. });
                if is_generator {
                    // Call __next__ on generator
                    let gen = iter_val.clone();
                    let next_func = gen.borrow().get_attribute("__next__");
                    if let Ok(next_func) = next_func {
                        // Fix self_obj by extracting name and func
                        let (n, f) = {
                            let b = next_func.borrow();
                            if let PyObject::BuiltinMethod { name, func, .. } = &*b {
                                (name.clone(), *func)
                            } else { return Err(PyError::runtime_error("expected __next__ method")) }
                        };
                        let fixed = PyObjectRef::new(PyObject::BuiltinMethod {
                            name: n,
                            func: f,
                            self_obj: gen.clone(),
                        });
                        match self.call_function(fixed, vec![]) {
                            Ok(val) => {
                                self.frames.last_mut().unwrap().push(val);
                            }
                            Err(e) if matches!(&e, PyError::StopIteration) => {
                                self.frames.last_mut().unwrap().ip = instr.arg as usize;
                            }
                            Err(e) => return Err(e),
                        }
                    } else {
                        self.frames.last_mut().unwrap().ip = instr.arg as usize;
                    }
                } else {
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
            }

            Opcode::LOAD_ATTR => {
                let name_idx = instr.arg as usize;
                let name = self.frames.last().unwrap().code.names.get(name_idx).ok_or_else(|| {
                    PyError::runtime_error("name index out of range")
                })?.clone();
                let obj = self.frames.last_mut().unwrap().pop()?;
                let result = {
                    let obj_borrowed = obj.borrow();
                    match &*obj_borrowed {
                        PyObject::Instance { dict, typ } => {
                            dict.get(&name).cloned().or_else(|| {
                                let typ_ref = typ.borrow();
                                if let PyObject::Type { dict: type_dict, mro, .. } = &*typ_ref {
                                    let found = type_dict.get(&name).cloned().or_else(|| {
                                        for base in mro.iter().skip(1) {
                                            if let PyObject::Type { dict: base_dict, .. } = &*base.borrow() {
                                                if let Some(val) = base_dict.get(&name) {
                                                    return Some(val.clone());
                                                }
                                            }
                                        }
                                        None
                                    });
                                    // Handle descriptor protocol for Property, StaticMethod, ClassMethod
                                    if let Some(val) = found {
                                        let val_borrowed = val.borrow();
                                        match &*val_borrowed {
                                            PyObject::Property { getter: Some(g), .. } => {
                                                drop(typ_ref);
                                                return Some(self.call_function(g.clone(), vec![obj.clone()]).unwrap_or_else(|_| val.clone()));
                                            }
                                            PyObject::StaticMethod { func } => {
                                                return Some(func.clone());
                                            }
                                            PyObject::ClassMethod { func } => {
                                                drop(typ_ref);
                                                let cls = obj.borrow();
                                                if let PyObject::Instance { typ: inst_typ, .. } = &*cls {
                                                    return Some(self.call_function(func.clone(), vec![inst_typ.clone()]).unwrap_or_else(|_| val.clone()));
                                                }
                                                return Some(val.clone());
                                            }
                                            PyObject::Function { .. } => {
                                                return Some(PyObjectRef::new(PyObject::BoundMethod {
                                                    func: val.clone(),
                                                    self_obj: obj.clone(),
                                                }));
                                            }
                                            _ => {
                                                return Some(val.clone());
                                            }
                                        }
                                    }
                                    None
                                } else {
                                    None
                                }
                            }).ok_or_else(|| PyError::attribute_error(format!("'{}' object has no attribute '{}'", obj_borrowed.type_name(), name)))
                        }
                        _ => {
                            let attr = obj_borrowed.get_attribute(&name).or_else(|_| {
                                Err(PyError::attribute_error(format!("'{}' object has no attribute '{}'", obj_borrowed.type_name(), name)))
                            })?;
                            drop(obj_borrowed);
                            let is_builtin_method = matches!(&*attr.borrow(), PyObject::BuiltinMethod { .. });
                            let is_function = matches!(&*attr.borrow(), PyObject::Function { .. });
                            if is_builtin_method {
                                let (n, func) = {
                                    let b = attr.borrow();
                                    if let PyObject::BuiltinMethod { name: n, func, .. } = &*b {
                                        (n.clone(), *func)
                                    } else { unreachable!() }
                                };
                                Ok(PyObjectRef::new(PyObject::BuiltinMethod {
                                    name: n,
                                    func,
                                    self_obj: obj.clone(),
                                }))
                            } else if is_function {
                                Ok(PyObjectRef::new(PyObject::BoundMethod {
                                    func: attr,
                                    self_obj: obj.clone(),
                                }))
                            } else {
                                Ok(attr)
                            }
                        }
                    }
                }?;
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

            Opcode::POP_EXCEPT => {
                self.frames.last_mut().unwrap().pop()?;
            }

            Opcode::CHECK_EXC_MATCH => {
                let expected = self.frames.last_mut().unwrap().pop()?;
                let exc = self.frames.last_mut().unwrap().pop()?;
                let expected_name = match &*expected.borrow() {
                    PyObject::Str(s) => s.clone(),
                    PyObject::Type { name, .. } => name.clone(),
                    PyObject::BuiltinFunction { name, .. } => name.clone(),
                    _ => return Err(PyError::type_error("exceptions must derive from BaseException")),
                };
                let matched = match &*exc.borrow() {
                    PyObject::Exception { typ, .. } => typ == &expected_name,
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
                    // Try to load from file
                    let module = self.import_module_from_file(&name);
                    match module {
                        Ok(m) => {
                            self.modules.insert(name.clone(), m.clone());
                            self.frames.last_mut().unwrap().push(m);
                        }
                        Err(_) => {
                            let module = create_module(&name, HashMap::new());
                            self.modules.insert(name.clone(), module.clone());
                            self.frames.last_mut().unwrap().push(module);
                        }
                    }
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
                self.frames.last_mut().unwrap().push(PyObjectRef::new(PyObject::BuildClass));
            }

            Opcode::LOAD_CLOSURE => {
                let idx = instr.arg as usize;
                let (_name, cell) = {
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
                let _spec = self.frames.last_mut().unwrap().pop()?;
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
                let _exit_method = {
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
                // Push sent value (None for next()) so execution can continue
                self.frames.last_mut().unwrap().push(py_none());
                return Ok(Some(val));
            }

            Opcode::RETURN_GENERATOR => {
                // Create a Generator wrapping current frame (IP already incremented past this instruction)
                let frame = self.frames.last().unwrap().clone();
                let gen = PyObjectRef::new(PyObject::Generator {
                    frame: std::cell::RefCell::new(Some(frame)),
                });
                // Push gen and return as if RETURN_VALUE — this exits execute()
                return Ok(Some(gen));
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

        if let PyObject::BoundMethod { func, self_obj } = &*callable.borrow() {
            let func = func.clone();
            let self_obj = self_obj.clone();
            let mut new_args = vec![self_obj];
            new_args.extend(args);
            return self.call_function(func, new_args);
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
            if !self.frames.is_empty() {
                self.frames.pop();
            }
            return result;
        }

        if let PyObject::Type { dict, mro, .. } = &*callable.borrow() {
            let instance = PyObjectRef::new(PyObject::Instance {
                typ: callable.clone(),
                dict: HashMap::new(),
            });
            let init_func = dict.get("__init__").cloned().or_else(|| {
                for base in mro.iter().skip(1) {
                    if let PyObject::Type { dict: base_dict, .. } = &*base.borrow() {
                        if let Some(val) = base_dict.get("__init__") {
                            return Some(val.clone());
                        }
                    }
                }
                None
            });
            if let Some(init_func) = init_func {
                let init_borrowed = init_func.borrow();
                match &*init_borrowed {
                    PyObject::BuiltinFunction { func, .. } => {
                        let func = *func;
                        let mut init_args = vec![instance.clone()];
                        init_args.extend(args);
                        func(&init_args)?;
                    }
                    PyObject::Function { code, globals: func_globals, .. } => {
                        let code = code.clone();
                        let func_globals = func_globals.clone();
                        drop(init_borrowed);
                        let mut new_frame = Frame::new(code, func_globals, self.builtins.clone());
                        new_frame.locals.insert(new_frame.code.varnames[0].clone(), instance.clone());
                        for (i, arg_name) in new_frame.code.varnames.iter().enumerate().skip(1) {
                            if i - 1 < args.len() {
                                new_frame.locals.insert(arg_name.clone(), args[i - 1].clone());
                            } else {
                                new_frame.locals.insert(arg_name.clone(), py_none());
                            }
                        }
                        self.frames.push(new_frame);
                        self.execute()?;
                        self.frames.pop();
                    }
                    _ => {}
                }
            }
            return Ok(instance);
        }

        if let PyObject::BuildClass = &*callable.borrow() {
            if args.len() < 3 {
                return Err(PyError::type_error("__build_class__: need at least 3 arguments"));
            }
            let func = args[0].clone();
            let name = args[1].clone();
            let bases = args[2].clone();

            let name_str = match &*name.borrow() {
                PyObject::Str(s) => s.clone(),
                _ => return Err(PyError::type_error("class name must be a string")),
            };

            let namespace = Rc::new(RefCell::new(HashMap::new()));

            match &*func.borrow() {
                PyObject::Function { code, .. } => {
                    let code = code.clone();
                    let new_frame = Frame::new(code, namespace.clone(), self.builtins.clone());
                    self.frames.push(new_frame);
                    self.execute()?;
                    self.frames.pop();
                }
                _ => return Err(PyError::type_error("class body must be a function")),
            }

            let namespace_dict = namespace.borrow().clone();

            let bases_vec = if matches!(&*bases.borrow(), PyObject::None) {
                vec![]
            } else if let PyObject::Tuple(t) = &*bases.borrow() {
                t.clone()
            } else {
                vec![bases.clone()]
            };

            let class = PyObjectRef::new(PyObject::Type {
                name: name_str,
                dict: namespace_dict,
                bases: bases_vec.clone(),
                mro: vec![],
            });

            let mut mro = vec![class.clone()];
            // C3 linearization for proper method resolution
            mro.extend(c3_linearize(&bases_vec));
            if let PyObject::Type { mro: mro_field, .. } = &mut *class.borrow_mut() {
                *mro_field = mro;
            }

            return Ok(class);
        }

        if let PyObject::Instance { typ, .. } = &*callable.borrow() {
            let f = {
                let typ_ref = typ.borrow();
                match &*typ_ref {
                    PyObject::Type { dict: type_dict, .. } => type_dict.get("__call__").cloned(),
                    _ => None,
                }
            };
            if let Some(f) = f {
                return self.call_function(f, args);
            }
        }

        Err(PyError::type_error(format!("'{}' object is not callable", type_name)))
    }

    fn handle_exception(&mut self, error: &PyError) -> bool {
        for frame in self.frames.iter_mut().rev() {
            while let Some(handler) = frame.exception_handlers.pop() {
                frame.stack.truncate(handler.stack_depth);
                frame.ip = handler.instr_addr;
                let exc_obj = {
                    let typ = match error {
                        PyError::TypeError(_) => "TypeError".to_string(),
                        PyError::ValueError(_) => "ValueError".to_string(),
                        PyError::NameError(_) => "NameError".to_string(),
                        PyError::AttributeError(_) => "AttributeError".to_string(),
                        PyError::IndexError(_) => "IndexError".to_string(),
                        PyError::KeyError(_) => "KeyError".to_string(),
                        PyError::ZeroDivisionError(_) => "ZeroDivisionError".to_string(),
                        PyError::RuntimeError(_) => "RuntimeError".to_string(),
                        PyError::StopIteration => "StopIteration".to_string(),
                        PyError::AssertionError(_) => "AssertionError".to_string(),
                        PyError::ImportError(_) => "ImportError".to_string(),
                        PyError::Exception(_, exc) => {
                            match &*exc.borrow() {
                                PyObject::Exception { typ, .. } => typ.clone(),
                                _ => "Exception".to_string(),
                            }
                        }
                        _ => "Exception".to_string(),
                    };
                    PyObjectRef::new(PyObject::Exception {
                        typ,
                        args: vec![py_str(&error.message())],
                        cause: None,
                    })
                };
                frame.push(exc_obj);
                return true;
            }
        }
        false
    }
}

fn c3_linearize(bases: &[PyObjectRef]) -> Vec<PyObjectRef> {
    if bases.is_empty() { return vec![]; }
    let mut result: Vec<PyObjectRef> = Vec::new();
    let mut remaining: Vec<Vec<PyObjectRef>> = Vec::new();
    for base in bases {
        let mut base_mro = vec![base.clone()];
        if let PyObject::Type { mro, .. } = &*base.borrow() {
            for cls in mro.iter() {
                if !result.iter().any(|c| c.borrow().type_name() == cls.borrow().type_name()) {
                    if !base_mro.iter().any(|c| c.borrow().type_name() == cls.borrow().type_name()) {
                        base_mro.push(cls.clone());
                    }
                }
            }
        }
        remaining.push(base_mro);
    }
    while !remaining.is_empty() {
        let mut found = false;
        for i in 0..remaining.len() {
            if remaining[i].is_empty() { continue; }
            let candidate = remaining[i][0].clone();
            let candidate_name = candidate.borrow().type_name();
            let mut bad = false;
            for j in 0..remaining.len() {
                if i == j { continue; }
                if remaining[j].len() > 1 {
                    for k in 1..remaining[j].len() {
                        if remaining[j][k].borrow().type_name() == candidate_name { bad = true; break; }
                    }
                }
                if bad { break; }
            }
            if !bad {
                result.push(candidate);
                for list in &mut remaining {
                    if !list.is_empty() && list[0].borrow().type_name() == candidate_name { list.remove(0); }
                }
                found = true;
                break;
            }
        }
        if !found { break; }
        remaining.retain(|l| !l.is_empty());
    }
    result
}
