use std::collections::HashMap;
use std::rc::Rc;
use std::cell::RefCell;
use num_bigint::BigInt;
use crate::bytecode::*;
use crate::object::*;
use crate::object::ObjectAccess;

#[derive(Clone)]
pub struct Frame {
    pub code: Rc<CodeObject>,
    pub locals: HashMap<String, PyObjectRef>,
    pub fast_locals: Vec<Option<PyObjectRef>>,
    pub globals: Rc<RefCell<HashMap<String, PyObjectRef>>>,
    pub builtins: Rc<HashMap<String, PyObjectRef>>,
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
        code: Rc<CodeObject>,
        globals: Rc<RefCell<HashMap<String, PyObjectRef>>>,
        builtins: Rc<HashMap<String, PyObjectRef>>,
    ) -> Self {
        Frame {
            fast_locals: vec![None; code.nlocals],
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
    pub builtins: Rc<HashMap<String, PyObjectRef>>,
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
              builtins: Rc::new(builtins),
              modules,
              globals,
          }
    }

    pub fn run(&mut self, code: CodeObject) -> PyResult<PyObjectRef> {
        let frame = Frame::new(
            Rc::new(code),
            self.globals.clone(),
            Rc::clone(&self.builtins),
        );
        self.frames.push(frame);
        let result = self.execute();
        self.frames.pop();
        result
    }

    pub fn exec_code(&mut self, code: CodeObject, globals: Option<Rc<RefCell<HashMap<String, PyObjectRef>>>>) -> PyResult<PyObjectRef> {
        let g = globals.unwrap_or_else(|| self.globals.clone());
        let frame = Frame::new(Rc::new(code), g, Rc::clone(&self.builtins));
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

    /// Try to execute a simple function without creating a Frame.
    /// Returns Some(result) if the function was simple enough, None otherwise.
    fn try_exec_simple(code: &CodeObject, args: &[PyObjectRef]) -> Option<PyResult<PyObjectRef>> {
        if code.vararg_name.is_some() || code.kwarg_name.is_some() || code.num_defaults > 0 {
            return None;
        }
        let instrs = &code.instructions;
        if instrs.is_empty() || instrs.len() > 6 {
            return None;
        }
        // Pre-allocate local variables from arguments
        let mut locals: Vec<Option<PyObjectRef>> = vec![None; code.varnames.len()];
        for (i, arg) in args.iter().enumerate() {
            if i < locals.len() {
                locals[i] = Some(arg.clone());
            }
        }
        let mut stack: Vec<PyObjectRef> = Vec::with_capacity(4);
        for instr in instrs {
            match instr.op {
                Opcode::LOAD_FAST => {
                    let idx = instr.arg as usize;
                    let val = locals.get(idx)?.clone()?;
                    stack.push(val);
                }
                Opcode::LOAD_CONST => {
                    let const_val = code.consts.get(instr.arg as usize)?;
                    let obj = match const_val {
                        ConstValue::None => py_none(),
                        ConstValue::Bool(b) => py_bool(*b),
                        ConstValue::Int(s) => {
                            if let Ok(n) = s.parse::<i64>() { py_int(n) }
                            else { let n: num_bigint::BigInt = s.parse().ok()?; PyObjectRef::new(PyObject::Int(n)) }
                        }
                        ConstValue::Float(s) => py_float(s.parse().ok()?),
                        ConstValue::String(s) => py_str(s),
                        _ => return None,
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
                        _ => return None,
                    };
                    match result { Ok(v) => stack.push(v), Err(e) => return Some(Err(e)) }
                }
                Opcode::RETURN_VALUE => return Some(Ok(stack.pop()?)),
                _ => return None,
            }
        }
        None
    }

    pub fn execute(&mut self) -> PyResult<PyObjectRef> {
        crate::object::VM_PTR.with(|p| *p.borrow_mut() = Some(self as *mut VirtualMachine));
        let result = self.execute_inner();
        crate::object::VM_PTR.with(|p| *p.borrow_mut() = None);
        result
    }

    fn execute_inner(&mut self) -> PyResult<PyObjectRef> {
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
        let fi = self.frames.len() - 1;
        let ip = self.frames[fi].ip;
        if ip >= self.frames[fi].code.instructions.len() {
            return Err(PyError::runtime_error("execution reached end of code"));
        }
        let op = self.frames[fi].code.instructions[ip].op;
        let arg = self.frames[fi].code.instructions[ip].arg;
        self.frames[fi].ip = ip + 1;

        match op {
            Opcode::NOP => {}

            Opcode::LOAD_CONST => {
                let const_idx = arg as usize;
                let const_val = self.frames[fi].code.consts.get(const_idx).ok_or_else(|| {
                    PyError::runtime_error(format!("constant index out of range: {}", const_idx))
                })?.clone();
                let obj = match const_val {
                    ConstValue::None => py_none(),
                    ConstValue::Bool(b) => py_bool(b),
                    ConstValue::Int(s) => {
                        if let Ok(n) = s.parse::<i64>() {
                            py_int(n)  // uses small int cache
                        } else {
                            let n: BigInt = s.parse().map_err(|_| {
                                PyError::value_error(format!("invalid integer: {}", s))
                            })?;
                            PyObjectRef::new(PyObject::Int(n))
                        }
                    }
                    ConstValue::Float(s) => {
                        let f: f64 = s.parse().map_err(|_| {
                            PyError::value_error(format!("invalid float: {}", s))
                        })?;
                        py_float(f)
                    }
                    ConstValue::String(s) => py_str(&s),
                    ConstValue::Bytes(b) => PyObjectRef::new(PyObject::Bytes(b)),
                    ConstValue::Code(code) => {
                        PyObjectRef::new(PyObject::Code(code))
                    }
                };
                self.frames[fi].push(obj);
            }

            Opcode::LOAD_NAME => {
                let name_idx = arg as usize;
                let name = self.frames[fi].code.names.get(name_idx).ok_or_else(|| {
                    PyError::runtime_error("name index out of range")
                })?.clone();
                let val = {
                    let f = &self.frames[self.frames.len() - 1];
                    f.locals.get(&name).cloned()
                        .or_else(|| f.globals.borrow().get(&name).cloned())
                        .or_else(|| f.builtins.get(&name).cloned())
                };
                match val {
                    Some(v) => self.frames[fi].push(v),
                    None => return Err(PyError::name_error(format!("name '{}' is not defined", name))),
                }
            }

            Opcode::STORE_NAME => {
                let name_idx = arg as usize;
                let name = self.frames[fi].code.names.get(name_idx).ok_or_else(|| {
                    PyError::runtime_error("name index out of range")
                })?.clone();
                let val = self.frames[fi].pop()?;
                self.frames[fi].globals.borrow_mut().insert(name, val);
            }

            Opcode::LOAD_FAST => {
                let var_idx = arg as usize;
                let val = {
                    let f = &self.frames[self.frames.len() - 1];
                    f.fast_locals.get(var_idx).and_then(|v| v.clone())
                };
                match val {
                    Some(v) => self.frames[fi].push(v),
                    None => return Err(PyError::name_error(format!("local variable '{}' referenced before assignment", 
                        self.frames[fi].code.varnames.get(var_idx).map_or("?", |s| &**s)))),
                }
            }

            Opcode::STORE_FAST => {
                let var_idx = arg as usize;
                let val = self.frames[fi].pop()?;
                let frame = &mut self.frames[fi];
                if var_idx < frame.fast_locals.len() {
                    frame.fast_locals[var_idx] = Some(val.clone());
                }
                let name = frame.code.varnames.get(var_idx).ok_or_else(|| {
                    PyError::runtime_error("varname index out of range")
                })?.clone();
                frame.locals.insert(name, val);
            }

            Opcode::LOAD_GLOBAL => {
                let name_idx = arg as usize;
                let name = self.frames[fi].code.names.get(name_idx).ok_or_else(|| {
                    PyError::runtime_error("name index out of range")
                })?.clone();
                let val = {
                    let f = &self.frames[self.frames.len() - 1];
                    f.globals.borrow().get(&name).cloned()
                        .or_else(|| f.builtins.get(&name).cloned())
                };
                match val {
                    Some(v) => self.frames[fi].push(v),
                    None => return Err(PyError::name_error(format!("name '{}' is not defined", name))),
                }
            }

            Opcode::STORE_GLOBAL => {
                let name_idx = arg as usize;
                let name = self.frames[fi].code.names.get(name_idx).ok_or_else(|| {
                    PyError::runtime_error("name index out of range")
                })?.clone();
                let val = self.frames[fi].pop()?;
                self.frames[fi].globals.borrow_mut().insert(name, val);
            }

            Opcode::LOAD_DEREF => {
                let var_idx = arg as usize;
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
                        self.frames[fi].push(push_val);
                    }
                    None => return Err(PyError::name_error(format!("variable '{}' not found", name))),
                }
            }

            Opcode::STORE_DEREF => {
                let var_idx = arg as usize;
                let name = {
                    let f = &self.frames[self.frames.len() - 1];
                    if var_idx < f.code.cellvars.len() {
                        f.code.cellvars[var_idx].clone()
                    } else {
                        f.code.freevars[var_idx - f.code.cellvars.len()].clone()
                    }
                };
                let val = self.frames[fi].pop()?;
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
                    self.frames[fi].locals.insert(name, cell_obj);
                }
            }

            Opcode::DELETE_FAST => {
                let var_idx = arg as usize;
                let name = self.frames[fi].code.varnames[var_idx].clone();
                self.frames[fi].locals.remove(&name);
            }

            Opcode::DELETE_NAME => {
                let name_idx = arg as usize;
                let name = self.frames[fi].code.names[name_idx].clone();
                self.frames[fi].globals.borrow_mut().remove(&name);
            }

            Opcode::POP_TOP => {
                self.frames[fi].pop()?;
            }

            Opcode::DUP_TOP => {
                let val = self.frames[fi].peek(0)?;
                self.frames[fi].push(val);
            }

            Opcode::COPY => {
                let depth = arg as usize;
                let val = self.frames[fi].peek(depth)?;
                self.frames[fi].push(val);
            }

            Opcode::SWAP => {
                let i = arg as usize;
                let len = self.frames[fi].stack.len();
                if i > 0 && i < len {
                    self.frames[fi].stack.swap(len - 1, len - 1 - i);
                }
            }

            Opcode::RETURN_VALUE => {
                let val = self.frames[fi].pop()?;
                return Ok(Some(val));
            }

            Opcode::PUSH_NULL => {
                self.frames[fi].push(py_none());
            }

            Opcode::CALL => {
                let npos = arg as usize & 0xFF;
                let nkw = (arg as usize >> 8) & 0xFF;
                let total = npos + 2 * nkw;
                let mut items = Vec::with_capacity(total);
                for _ in 0..total {
                    items.push(self.frames[fi].pop()?);
                }
                items.reverse();
                let mut args = Vec::with_capacity(npos + nkw);
                for i in 0..npos {
                    args.push(items[i].clone());
                }
                let mut keywords = Vec::new();
                for i in 0..nkw {
                    let name = match &*items[npos + 2 * i].borrow() {
                        PyObject::Str(s) => s.clone(),
                        _ => return Err(PyError::type_error("keyword must be a string")),
                    };
                    let value = items[npos + 2 * i + 1].clone();
                    keywords.push((name, value));
                }
                let callable = self.frames[fi].pop()?;
                let result = self.call_function(callable, args, keywords)?;
                self.frames[fi].push(result);
            }

            Opcode::MAKE_FUNCTION => {
                let n_defaults = arg as usize;
                let mut defaults = Vec::new();
                for _ in 0..n_defaults {
                    defaults.push(self.frames[fi].pop()?);
                }
                defaults.reverse();
                let code_obj = self.frames[fi].pop()?;
                let code = match &*code_obj.borrow() {
                    PyObject::Code(c) => Rc::new(c.as_ref().clone()),
                    _ => return Err(PyError::runtime_error("MAKE_FUNCTION: expected code object")),
                };
                let globals = self.frames[fi].globals.clone();
                let func = PyObjectRef::new(PyObject::Function {
                    code,
                    globals,
                    name: "<function>".to_string(),
                    defaults,
                    closure: Vec::new(),
                    dict: HashMap::new(),
                });
                self.frames[fi].push(func);
            }

            Opcode::BUILD_LIST => {
                let count = arg as usize;
                let mut items = Vec::with_capacity(count);
                for _ in 0..count {
                    items.push(self.frames[fi].pop()?);
                }
                items.reverse();
                self.frames[fi].push(py_list(items));
            }

            Opcode::BUILD_TUPLE => {
                let count = arg as usize;
                let mut items = Vec::with_capacity(count);
                for _ in 0..count {
                    items.push(self.frames[fi].pop()?);
                }
                items.reverse();
                self.frames[fi].push(py_tuple(items));
            }

            Opcode::BUILD_MAP => {
                self.frames[fi].push(py_dict());
            }

            Opcode::BUILD_SET => {
                let count = arg as usize;
                let mut items = Vec::with_capacity(count);
                for _ in 0..count {
                    items.push(self.frames[fi].pop()?);
                }
                items.reverse();
                self.frames[fi].push(PyObjectRef::new(PyObject::Set(items)));
            }

            Opcode::BUILD_STRING => {
                let count = arg as usize;
                let mut parts = Vec::with_capacity(count);
                for _ in 0..count {
                    parts.push(self.frames[fi].pop()?.str());
                }
                parts.reverse();
                self.frames[fi].push(py_str(&parts.join("")));
            }

            Opcode::BUILD_SLICE => {
                let nargs = arg as usize;
                let step = if nargs >= 3 { Some(self.frames[fi].pop()?) } else { None };
                let stop = if nargs >= 2 { Some(self.frames[fi].pop()?) } else { None };
                let start = if nargs >= 1 { Some(self.frames[fi].pop()?) } else { None };
                self.frames[fi].push(PyObjectRef::new(PyObject::Slice {
                    start: start.unwrap_or(py_none()),
                    stop: stop.unwrap_or(py_none()),
                    step: step.unwrap_or(py_none()),
                }));
            }

            Opcode::BINARY_OP => {
                let op = arg;
                let right = self.frames[fi].pop()?;
                let left = self.frames[fi].pop()?;
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
                self.frames[fi].push(result);
            }

            Opcode::COMPARE_OP => {
                let op = arg;
                let right = self.frames[fi].pop()?;
                let left = self.frames[fi].pop()?;
                let result = py_compare(&left, &right, op)?;
                self.frames[fi].push(result);
            }

            Opcode::IS_OP => {
                let invert = arg != 0;
                let right = self.frames[fi].pop()?;
                let left = self.frames[fi].pop()?;
                let is_same = left.is(&right);
                let result = if invert { !is_same } else { is_same };
                self.frames[fi].push(py_bool(result));
            }

            Opcode::CONTAINS_OP => {
                let invert = arg != 0;
                let right = self.frames[fi].pop()?;
                let left = self.frames[fi].pop()?;
                let result = contains_op(&right, &left)?;
                let result = if invert { !result } else { result };
                self.frames[fi].push(py_bool(result));
            }

            Opcode::UNARY_NEGATIVE => {
                let val = self.frames[fi].pop()?;
                let result = py_sub(&py_int(0), &val)?;
                self.frames[fi].push(result);
            }

            Opcode::UNARY_NOT => {
                let val = self.frames[fi].pop()?;
                self.frames[fi].push(py_bool(!val.truthy()));
            }

            Opcode::UNARY_INVERT => {
                let val = self.frames[fi].pop()?;
                let result = {
                    let obj = val.borrow();
                    match &*obj {
                        PyObject::Int(i) => py_int(!i),
                        _ => return Err(PyError::type_error("bad operand type for unary ~")),
                    }
                };
                self.frames[fi].push(result);
            }

            Opcode::JUMP_FORWARD | Opcode::JUMP | Opcode::JUMP_BACKWARD => {
                let offset = arg as usize;
                match op {
                    Opcode::JUMP_FORWARD => {
                        self.frames[fi].ip += offset;
                    }
                    Opcode::JUMP => {
                        self.frames[fi].ip = offset;
                    }
                    Opcode::JUMP_BACKWARD => {
                        let cur_ip = self.frames[fi].ip;
                        self.frames[fi].ip = cur_ip.wrapping_sub(offset).wrapping_sub(1);
                    }
                    _ => unreachable!(),
                }
            }

            Opcode::POP_JUMP_IF_FALSE => {
                let val = self.frames[fi].pop()?;
                if !val.truthy() {
                    self.frames[fi].ip = arg as usize;
                }
            }

            Opcode::POP_JUMP_IF_TRUE => {
                let val = self.frames[fi].pop()?;
                if val.truthy() {
                    self.frames[fi].ip = arg as usize;
                }
            }

            Opcode::POP_JUMP_IF_NONE => {
                let val = self.frames[fi].pop()?;
                let is_none = {
                    matches!(&*val.borrow(), PyObject::None)
                };
                if is_none {
                    self.frames[fi].ip = arg as usize;
                }
            }

            Opcode::POP_JUMP_IF_NOT_NONE => {
                let val = self.frames[fi].pop()?;
                let is_not_none = {
                    !matches!(&*val.borrow(), PyObject::None)
                };
                if is_not_none {
                    self.frames[fi].ip = arg as usize;
                }
            }

            Opcode::GET_ITER => {
                let val = self.frames[fi].pop()?;
                let obj = val.borrow();
                match &*obj {
                    PyObject::List(v) => {
                        self.frames[fi].push(PyObjectRef::new(PyObject::ListIter { list: v.clone(), index: 0 }));
                    }
                    PyObject::Tuple(v) => {
                        self.frames[fi].push(PyObjectRef::new(PyObject::ListIter { list: v.clone(), index: 0 }));
                    }
                    PyObject::Str(s) => {
                        let chars: Vec<PyObjectRef> = s.chars().map(|c| py_str(&c.to_string())).collect();
                        self.frames[fi].push(PyObjectRef::new(PyObject::ListIter { list: chars, index: 0 }));
                    }
                    PyObject::Set(s) => {
                        self.frames[fi].push(PyObjectRef::new(PyObject::ListIter { list: s.clone(), index: 0 }));
                    }
                    PyObject::Generator { .. } => {
                        drop(obj);
                        self.frames[fi].push(val);
                    }
                    PyObject::Range { start, stop, step } => {
                        self.frames[fi].push(PyObjectRef::new(PyObject::RangeIter { current: *start, stop: *stop, step: *step }));
                    }
                    _ => return Err(PyError::type_error(format!("'{}' object is not iterable", obj.type_name()))),
                }
            }

            Opcode::FOR_ITER => {
                let iter_val = self.frames[fi].peek(0)?;
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
                        match self.call_function(fixed, vec![], vec![]) {
                            Ok(val) => {
                                self.frames[fi].push(val);
                            }
                            Err(e) if matches!(&e, PyError::StopIteration) => {
                                self.frames[fi].ip = arg as usize;
                            }
                            Err(e) => return Err(e),
                        }
                    } else {
                        self.frames[fi].ip = arg as usize;
                    }
                } else {
                let is_exhausted = {
                    let obj = iter_val.borrow();
                    match &*obj {
                        PyObject::List(v) => v.is_empty(),
                        PyObject::ListIter { list, index } => *index >= list.len(),
                        PyObject::RangeIter { current, stop, step } => {
                            if *step > 0 { *current >= *stop } else { *current <= *stop }
                        }
                        _ => return Err(PyError::type_error("for_iter on non-iterable")),
                    }
                };
                if is_exhausted {
                    self.frames[fi].ip = arg as usize;
                } else {
                    let val = self.frames[fi].pop()?;
                    let item = {
                        let mut obj = val.borrow_mut();
                        match &mut *obj {
                            PyObject::List(list) => list.remove(0),
                            PyObject::ListIter { list, index } => {
                                let v = list[*index].clone();
                                *index += 1;
                                v
                            }
                            PyObject::RangeIter { current, stop, step } => {
                                let v = py_int(*current);
                                *current += *step;
                                v
                            }
                            _ => unreachable!()
                        }
                    };
                    self.frames[fi].push(val);
                    self.frames[fi].push(item);
                }
                }
            }

            Opcode::LOAD_ATTR => {
                let name_idx = arg as usize;
                let name = self.frames[fi].code.names.get(name_idx).ok_or_else(|| {
                    PyError::runtime_error("name index out of range")
                })?.clone();
                let obj = self.frames[fi].pop()?;
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
                                                return Some(self.call_function(g.clone(), vec![obj.clone()], vec![]).unwrap_or_else(|_| val.clone()));
                                            }
                                            PyObject::StaticMethod { func } => {
                                                return Some(func.clone());
                                            }
                                            PyObject::ClassMethod { func } => {
                                                drop(typ_ref);
                                                let cls = obj.borrow();
                                                if let PyObject::Instance { typ: inst_typ, .. } = &*cls {
                                                    return Some(self.call_function(func.clone(), vec![inst_typ.clone()], vec![]).unwrap_or_else(|_| val.clone()));
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
                self.frames[fi].push(result);
            }

            Opcode::STORE_ATTR => {
                let name_idx = arg as usize;
                let name = self.frames[fi].code.names.get(name_idx).ok_or_else(|| {
                    PyError::runtime_error("name index out of range")
                })?.clone();
                let val = self.frames[fi].pop()?;
                let obj = self.frames[fi].pop()?;
                obj.borrow_mut().set_attribute(&name, val)?;
            }

            Opcode::STORE_SUBSCR => {
                let val = self.frames[fi].pop()?;
                let index = self.frames[fi].pop()?;
                let obj = self.frames[fi].pop()?;
                py_setitem(&obj, &index, val)?;
            }

            Opcode::LIST_APPEND => {
                let val = self.frames[fi].pop()?;
                let list = self.frames[fi].peek(arg as usize)?;
                let mut obj = list.borrow_mut();
                if let PyObject::List(v) = &mut *obj {
                    v.push(val);
                } else {
                    return Err(PyError::runtime_error("LIST_APPEND on non-list"));
                }
            }

            Opcode::SET_ADD => {
                let val = self.frames[fi].pop()?;
                let set = self.frames[fi].peek(arg as usize)?;
                let mut obj = set.borrow_mut();
                if let PyObject::Set(v) = &mut *obj {
                    v.push(val);
                } else {
                    return Err(PyError::runtime_error("SET_ADD on non-set"));
                }
            }

            Opcode::MAP_ADD => {
                let val = self.frames[fi].pop()?;
                let key = self.frames[fi].pop()?;
                let map = self.frames[fi].peek(arg as usize)?;
                let mut obj = map.borrow_mut();
                if let PyObject::Dict(d) = &mut *obj {
                    let key_str = key.str();
                    d.insert(key_str, val);
                } else {
                    return Err(PyError::runtime_error("MAP_ADD on non-dict"));
                }
            }

            Opcode::UNPACK_SEQUENCE => {
                let count = arg as usize;
                let seq = self.frames[fi].pop()?;
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
                    self.frames[fi].push(item);
                }
            }

            Opcode::SETUP_FINALLY => {
                let stack_depth = self.frames[fi].stack.len();
                let handler = ExceptionHandler {
                    instr_addr: arg as usize,
                    stack_depth,
                    handler_type: HandlerType::Except,
                };
                self.frames[fi].exception_handlers.push(handler);
            }

            Opcode::SETUP_CLEANUP => {
                let stack_depth = self.frames[fi].stack.len();
                let handler = ExceptionHandler {
                    instr_addr: arg as usize,
                    stack_depth,
                    handler_type: HandlerType::Finally,
                };
                self.frames[fi].exception_handlers.push(handler);
            }

            Opcode::POP_BLOCK => {
                self.frames[fi].exception_handlers.pop();
            }

            Opcode::PUSH_EXC_INFO => {}

            Opcode::POP_EXCEPT => {
                self.frames[fi].pop()?;
            }

            Opcode::CHECK_EXC_MATCH => {
                let expected = self.frames[fi].pop()?;
                let exc = self.frames[fi].pop()?;
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
                self.frames[fi].push(py_bool(matched));
            }

            Opcode::RERAISE => {
                return Err(PyError::runtime_error("re-raise"));
            }

            Opcode::RAISE_VARARGS => {
                let nargs = arg;
                match nargs {
                    0 => return Err(PyError::runtime_error("re-raise")),
                    1 => {
                        let exc = self.frames[fi].pop()?;
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
                        let cause = self.frames[fi].pop()?;
                        let exc = self.frames[fi].pop()?;
                        return Err(PyError::Exception(format!("{} (caused by {})", exc.str(), cause.str()), exc));
                    }
                    _ => return Err(PyError::runtime_error("invalid RAISE_VARARGS count")),
                }
            }

            Opcode::IMPORT_NAME => {
                let name_idx = arg as usize;
                let name = self.frames[fi].code.names.get(name_idx).ok_or_else(|| {
                    PyError::runtime_error("name index out of range")
                })?.clone();
                self.frames[fi].pop()?;
                self.frames[fi].pop()?;
                if self.modules.contains_key(&name) {
                    self.frames[fi].push(self.modules[&name].clone());
                } else {
                    // Try to load from file
                    let module = self.import_module_from_file(&name);
                    match module {
                        Ok(m) => {
                            self.modules.insert(name.clone(), m.clone());
                            self.frames[fi].push(m);
                        }
                        Err(_) => {
                            let module = create_module(&name, HashMap::new());
                            self.modules.insert(name.clone(), module.clone());
                            self.frames[fi].push(module);
                        }
                    }
                }
            }

            Opcode::IMPORT_FROM => {
                let name_idx = arg as usize;
                let name = self.frames[fi].code.names.get(name_idx).ok_or_else(|| {
                    PyError::runtime_error("name index out of range")
                })?.clone();
                let module = self.frames[fi].peek(0)?;
                let obj = module.borrow();
                match &*obj {
                    PyObject::Module { dict, .. } => {
                        if let Some(val) = dict.get(&name) {
                            self.frames[fi].push(val.clone());
                        } else {
                            self.frames[fi].push(py_none());
                        }
                    }
                    _ => return Err(PyError::runtime_error("IMPORT_FROM on non-module")),
                }
            }

            Opcode::LOAD_BUILD_CLASS => {
                self.frames[fi].push(PyObjectRef::new(PyObject::BuildClass));
            }

            Opcode::LOAD_CLOSURE => {
                let idx = arg as usize;
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
                self.frames[fi].push(cell);
            }

            Opcode::FORMAT_SIMPLE => {
                let val = self.frames[fi].pop()?;
                self.frames[fi].push(py_str(&val.str()));
            }

            Opcode::FORMAT_WITH_SPEC => {
                let _spec = self.frames[fi].pop()?;
                let val = self.frames[fi].pop()?;
                self.frames[fi].push(py_str(&val.str()));
            }

            Opcode::CONVERT_VALUE => {
                let conversion = arg;
                let val = self.frames[fi].pop()?;
                let result = match conversion {
                    0 => py_str(&val.str()),
                    1 => py_str(&val.repr()),
                    2 => py_str(&val.str()),
                    _ => return Err(PyError::runtime_error("unknown conversion type")),
                };
                self.frames[fi].push(result);
            }

            Opcode::LOAD_LOCALS => {
                self.frames[fi].push(py_dict());
            }

            Opcode::SETUP_ANNOTATIONS => {}

            Opcode::POP_ITER => {
                self.frames[fi].pop()?;
            }

            Opcode::SETUP_WITH => {
                // Simplified: just enter the context manager
                let mgr = self.frames[fi].peek(0)?;
                let _exit_method = {
                    let obj = mgr.borrow();
                    obj.get_attribute("__exit__").ok()
                };
                let enter_method = {
                    let obj = mgr.borrow();
                    obj.get_attribute("__enter__").ok()
                };
                if let Some(enter) = enter_method {
                    let result = self.call_function(enter, vec![], vec![])?;
                    self.frames[fi].push(result);
                } else {
                    // Enter and push None as result (simplified)
                    self.frames[fi].push(py_none());
                }
            }

            Opcode::YIELD_VALUE => {
                let val = self.frames[fi].pop()?;
                // Push sent value (None for next()) so execution can continue
                self.frames[fi].push(py_none());
                return Ok(Some(val));
            }

            Opcode::RETURN_GENERATOR => {
                // Create a Generator wrapping current frame (IP already incremented past this instruction)
                let frame = self.frames[fi].clone();
                let gen = PyObjectRef::new(PyObject::Generator {
                    frame: std::cell::RefCell::new(Some(frame)),
                });
                // Push gen and return as if RETURN_VALUE — this exits execute()
                return Ok(Some(gen));
            }

            _ => return Err(PyError::runtime_error(format!("unimplemented opcode: {:?}", op))),
        }
        Ok(None)
    }

    fn call_function(&mut self, callable: PyObjectRef, args: Vec<PyObjectRef>, keywords: Vec<(String, PyObjectRef)>) -> PyResult<PyObjectRef> {
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
            return self.call_function(func, new_args, keywords);
        }

        if let PyObject::Function { code, globals: func_globals, defaults, .. } = &*callable.borrow() {
            // Try simple execution without Frame creation
            if defaults.is_empty() && keywords.is_empty() {
                if let Some(result) = Self::try_exec_simple(code, &args) {
                    return result;
                }
            }
            let code = code.clone();
            let func_globals = func_globals.clone();
            let defaults = defaults.clone();
            let mut new_frame = Frame::new(code.clone(), func_globals, Rc::clone(&self.builtins));

            let npos = args.len();
            let named_params = if code.vararg_name.is_some() || code.kwarg_name.is_some() {
                code.varnames.iter().position(|n| {
                    Some(n.clone()) == code.vararg_name || Some(n.clone()) == code.kwarg_name
                }).unwrap_or(code.varnames.len())
            } else {
                code.varnames.len()
            };

            // Assign positional args to named parameters
            for i in 0..npos.min(named_params) {
                let arg_name = &new_frame.code.varnames[i];
                new_frame.locals.insert(arg_name.clone(), args[i].clone());
                if i < new_frame.fast_locals.len() {
                    new_frame.fast_locals[i] = Some(args[i].clone());
                }
            }

            // Pack excess positional args into *args
            if let Some(vararg_name) = &code.vararg_name {
                let mut extra = Vec::new();
                for i in named_params..npos {
                    extra.push(args[i].clone());
                }
                let vararg_val = py_tuple(extra);
                if let Some(idx) = new_frame.code.varnames.iter().position(|n| n == vararg_name) {
                    if idx < new_frame.fast_locals.len() {
                        new_frame.fast_locals[idx] = Some(vararg_val.clone());
                    }
                }
                new_frame.locals.insert(vararg_name.clone(), vararg_val);
            }

            // Apply defaults for missing positional params
            if npos < named_params {
                let num_defaults = code.num_defaults;
                for i in npos..named_params {
                    let default_idx = num_defaults - (named_params - i);
                    let arg_name = &new_frame.code.varnames[i];
                    let val = if default_idx < defaults.len() {
                        defaults[default_idx].clone()
                    } else {
                        py_none()
                    };
                    new_frame.locals.insert(arg_name.clone(), val.clone());
                    if i < new_frame.fast_locals.len() {
                        new_frame.fast_locals[i] = Some(val);
                    }
                }
            }

            // Handle **kwargs
            if let Some(kwarg_name) = &code.kwarg_name {
                let mut kw_dict = py_dict();
                for (key, value) in &keywords {
                    if let Some(idx) = new_frame.code.varnames.iter().position(|n| n == key) {
                        new_frame.locals.insert(key.clone(), value.clone());
                        if idx < new_frame.fast_locals.len() {
                            new_frame.fast_locals[idx] = Some(value.clone());
                        }
                    } else {
                        if let PyObject::Dict(ref mut dict) = &mut *kw_dict.borrow_mut() {
                            dict.insert(key.clone(), value.clone());
                        }
                    }
                }
                if let Some(idx) = new_frame.code.varnames.iter().position(|n| n == kwarg_name) {
                    if idx < new_frame.fast_locals.len() {
                        new_frame.fast_locals[idx] = Some(kw_dict.clone());
                    }
                }
                new_frame.locals.insert(kwarg_name.clone(), kw_dict);
            } else {
                // No **kwargs, just set keyword args directly
                for (key, value) in &keywords {
                    new_frame.locals.insert(key.clone(), value.clone());
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
                        let mut new_frame = Frame::new(code, func_globals, Rc::clone(&self.builtins));
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
                    let new_frame = Frame::new(code, namespace.clone(), Rc::clone(&self.builtins));
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
                return self.call_function(f, args, vec![]);
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
