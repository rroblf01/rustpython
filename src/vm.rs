use std::collections::HashMap;
use std::rc::Rc;
use std::cell::RefCell;
use num_bigint::BigInt;
use smallvec::SmallVec;
use crate::bytecode::*;
use crate::object::*;
use crate::object::ObjectAccess;
use crate::jit::JitCompiler;

thread_local! {
    static ATTR_CACHE: std::cell::RefCell<HashMap<(String, String), crate::object::BuiltinFunc>> = std::cell::RefCell::new(HashMap::new());
}

#[derive(Clone)]
pub struct Frame {
    pub code: Rc<CodeObject>,
    pub locals: HashMap<String, PyObjectRef>,
    pub fast_locals: Vec<Option<PyObjectRef>>,
    pub globals: Rc<RefCell<HashMap<String, PyObjectRef>>>,
    pub builtins: Rc<HashMap<String, PyObjectRef>>,
    pub stack: SmallVec<[PyObjectRef; 8]>,
    pub ip: usize,
    pub base_sp: usize,
    pub exception_handlers: Vec<ExceptionHandler>,
    pub return_value: Option<PyResult<PyObjectRef>>,
    pub closure: Vec<PyObjectRef>,
    /// Inline attribute cache — caches LOAD_ATTR results per instruction offset.
    /// Cleared when the frame is created; populated on first attribute access.
    pub attr_cache: Vec<Option<(u64, PyObjectRef)>>,  // (type_version_tag, cached_value)
    /// Inline global cache — caches LOAD_GLOBAL results per instruction offset.
    pub global_cache: Vec<Option<PyObjectRef>>,
    /// Virtual registers for register-based bytecode execution.
    /// 256 virtual registers (u8 index) — no stack needed for most ops.
    pub registers: Vec<Option<PyObjectRef>>,
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
            stack: SmallVec::new(),
            ip: 0,
            base_sp: 0,
            exception_handlers: Vec::new(),
            return_value: None,
            closure: Vec::new(),
            attr_cache: Vec::new(),
            global_cache: Vec::new(),
            registers: Vec::new(),
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
    pub jit: RefCell<JitCompiler>,
    pub sys_path: Vec<String>,
    /// Execution profile counters — how many times each instruction ran.
    /// Indexed by (function_id, instruction_offset). Used by JIT to
    /// identify hot paths for native compilation.
    pub profile: RefCell<HashMap<usize, Vec<u32>>>,
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

         let sys_dict = create_sys_dict(std::env::args().collect());
         modules.insert("sys".to_string(), create_module("sys", sys_dict.clone()));
          builtins.extend(sys_dict.clone());

         let os_dict = create_os_dict();
         modules.insert("os".to_string(), create_module("os", os_dict.clone()));

         let json_dict = create_json_dict();
         modules.insert("json".to_string(), create_module("json", json_dict));

         let collections_dict = create_collections_dict();
          modules.insert("collections".to_string(), create_module("collections", collections_dict));

          let functools_dict = create_functools_dict();
          modules.insert("functools".to_string(), create_module("functools", functools_dict));

          let itertools_dict = create_itertools_dict();
          modules.insert("itertools".to_string(), create_module("itertools", itertools_dict));

          let random_dict = create_random_dict();
          modules.insert("random".to_string(), create_module("random", random_dict));

          let datetime_dict = create_datetime_dict();
          modules.insert("datetime".to_string(), create_module("datetime", datetime_dict));

          let socket_dict = create_socket_dict();
          modules.insert("socket".to_string(), create_module("socket", socket_dict));

          let select_dict = create_select_dict();
          modules.insert("select".to_string(), create_module("select", select_dict));

          let re_dict = create_re_dict();
          modules.insert("re".to_string(), create_module("re", re_dict));

          let subprocess_dict = create_subprocess_dict();
          modules.insert("subprocess".to_string(), create_module("subprocess", subprocess_dict));

          let threading_dict = create_threading_dict();
          modules.insert("threading".to_string(), create_module("threading", threading_dict));

          // Populate sys.path with default search paths
         if let PyObject::List(path_list) = &mut *sys_dict.get("path").unwrap().borrow_mut() {
             path_list.push(py_str("."));
             path_list.push(py_str("./Lib"));
         }
         // Populate sys.modules with already-loaded modules
         if let PyObject::Dict(mod_dict) = &mut *sys_dict.get("modules").unwrap().borrow_mut() {
             for (name, module) in &modules {
                 mod_dict.set(py_str(name), module.clone()).ok();
             }
         }
 
         VirtualMachine {
              frames: Vec::new(),
              builtins: Rc::new(builtins),
              modules,
              globals,
              jit: RefCell::new(JitCompiler::new()),
              sys_path: vec!["./".to_string(), "./Lib/".to_string()],
              profile: RefCell::new(HashMap::new()),
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

    fn import_module_from_file(&mut self, name: &str) -> Result<PyObjectRef, String> {
        let search_paths = self.get_sys_path();
        for base in &search_paths {
            let py_path = if base.ends_with('/') {
                format!("{}{}.py", base, name)
            } else {
                format!("{}/{}.py", base, name)
            };
            if let Ok(source) = std::fs::read_to_string(&py_path) {
                return self.exec_module_source(&source, &py_path, name);
            }
            let init_path = if base.ends_with('/') {
                format!("{}{}/__init__.py", base, name)
            } else {
                format!("{}/{}/__init__.py", base, name)
            };
            if let Ok(source) = std::fs::read_to_string(&init_path) {
                return self.exec_module_source(&source, &init_path, name);
            }
        }
        Err(format!("No module named '{}'", name))
    }

    fn get_sys_path(&self) -> Vec<String> {
        if let Some(sys_mod) = self.modules.get("sys") {
            if let PyObject::Module { dict, .. } = &*sys_mod.borrow() {
                if let Some(path_list) = dict.get("path") {
                    if let PyObject::List(items) = &*path_list.borrow() {
                        return items.iter().filter_map(|item| {
                            if let PyObject::Str(s) = &*item.borrow() { Some(s.clone()) } else { None }
                        }).collect();
                    }
                }
            }
        }
        vec![]
    }

    fn exec_module_source(&mut self, source: &str, path: &str, name: &str) -> Result<PyObjectRef, String> {
        let mut parser = crate::parser::Parser::new(source);
        let program = parser.parse_program().map_err(|e| format!("Parse error: {}", e))?;
        let mut compiler = crate::compiler::Compiler::new();
        let code = compiler.compile(&program, path).map_err(|e| format!("Compile error: {}", e))?;
        let module_globals = Rc::new(RefCell::new(HashMap::from([
            ("__name__".to_string(), py_str(name)),
            ("__file__".to_string(), py_str(path)),
            ("__builtins__".to_string(), create_module("builtins", self.builtins.as_ref().clone())),
        ])));
        self.exec_code(code, Some(Rc::clone(&module_globals))).map_err(|e| format!("{}", e))?;
        let globals_copy = module_globals.borrow().clone();
        Ok(create_module(name, globals_copy))
    }

    /// Try to execute a simple function without creating a Frame.
    /// Returns Some(result) if the function was simple enough, None otherwise.
    fn try_exec_simple(code: &CodeObject, args: &[PyObjectRef]) -> Option<PyResult<PyObjectRef>> {
        if code.vararg_name.is_some() || code.kwarg_name.is_some() || code.num_defaults > 0 {
            return None;
        }
        let instrs = &code.instructions;
        if instrs.is_empty() || instrs.len() > 12 {
            return None;
        }
        // Pre-allocate local variables from arguments
        let mut locals: Vec<Option<PyObjectRef>> = vec![None; code.varnames.len()];
        for (i, arg) in args.iter().enumerate() {
            if i < locals.len() {
                locals[i] = Some(arg.clone());
            }
        }
        let mut stack: SmallVec<[PyObjectRef; 8]> = SmallVec::new();
        let mut ip: usize = 0;
        let n_instrs = instrs.len();
        loop {
            if ip >= n_instrs { return None; }
            let instr = &instrs[ip];
            ip += 1;
            match instr.op {
                Opcode::LOAD_FAST => {
                    let idx = instr.arg as usize;
                    let val = locals.get(idx)?.clone()?;
                    stack.push(val);
                }
                Opcode::STORE_FAST => {
                    let idx = instr.arg as usize;
                    let val = stack.pop()?;
                    if idx < locals.len() { locals[idx] = Some(val); }
                }
                Opcode::LOAD_CONST => {
                    let const_val = code.consts.get(instr.arg as usize)?;
                    let obj = match const_val {
                        ConstValue::None => py_none(),
                        ConstValue::Bool(b) => py_bool(*b),
                        ConstValue::Int(s) => {
                            if let Ok(n) = s.parse::<i64>() { py_int(n) }
                            else { let n: num_bigint::BigInt = s.parse().ok()?; PyObjectRef::imm(PyObject::Int(n)) }
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
                        4 => py_floor_div(&left, &right),
                        5 => py_mod(&left, &right),
                        6 => py_pow(&left, &right),
                        7 => py_lshift(&left, &right),
                        8 => py_rshift(&left, &right),
                        9 => py_bit_or(&left, &right),
                        10 => py_bit_and(&left, &right),
                        11 => py_bit_xor(&left, &right),
                        13 => py_getitem(&left, &right),
                        _ => return None,
                    };
                    match result { Ok(v) => stack.push(v), Err(e) => return Some(Err(e)) }
                }
                Opcode::COMPARE_OP => {
                    let right = stack.pop()?;
                    let left = stack.pop()?;
                    let result = py_compare(&left, &right, instr.arg);
                    match result { Ok(v) => stack.push(v), Err(e) => return Some(Err(e)) }
                }
                Opcode::POP_JUMP_IF_FALSE => {
                    let val = stack.pop()?;
                    if !val.truthy() { ip = instr.arg as usize; }
                }
                Opcode::JUMP_FORWARD => {
                    ip = ip + instr.arg as usize;
                }
                Opcode::JUMP_BACKWARD => {
                    ip = ip - (instr.arg as usize + 1);
                }
                Opcode::RETURN_VALUE => return Some(Ok(stack.pop()?)),
                _ => return None,
            }
        }
    }

    pub fn execute(&mut self) -> PyResult<PyObjectRef> {
        crate::object::VM_PTR.with(|p| {
            if p.borrow().is_none() {
                *p.borrow_mut() = Some(self as *mut VirtualMachine);
            }
        });
        self.execute_inner()
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

    #[inline(always)]
    fn execute_instruction(&mut self) -> PyResult<Option<PyObjectRef>> {
        let fi = self.frames.len() - 1;
        let ip = self.frames[fi].ip;
        if ip >= self.frames[fi].code.instructions.len() {
            return Err(PyError::runtime_error("execution reached end of code"));
        }
        let op = self.frames[fi].code.instructions[ip].op;
        let arg = self.frames[fi].code.instructions[ip].arg;
        self.frames[fi].ip = ip + 1;

        // Profile: increment counter for this instruction
        // Only in profile mode (disabled by default for speed)
        if cfg!(feature = "profile") {
            let func_id = fi; // use frame index as function identifier
            let mut prof = self.profile.borrow_mut();
            let counters = prof.entry(func_id).or_insert_with(|| vec![0u32; self.frames[fi].code.instructions.len()]);
            if ip < counters.len() {
                counters[ip] = counters[ip].saturating_add(1);
            }
        }

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
                            PyObjectRef::imm(PyObject::Int(n))
                        }
                    }
                    ConstValue::Float(s) => {
                        let f: f64 = s.parse().map_err(|_| {
                            PyError::value_error(format!("invalid float: {}", s))
                        })?;
                        py_float(f)
                    }
                    ConstValue::String(s) => py_str(&s),
                    ConstValue::Bytes(b) => PyObjectRef::imm(PyObject::Bytes(b)),
                    ConstValue::Code(code) => {
                        PyObjectRef::imm(PyObject::Code(code))
                    }
                };
                self.frames[fi].push(obj);
            }

            Opcode::LOAD_NAME => {
                let name_idx = arg as usize;
                let name = &self.frames[fi].code.names[name_idx];
                let val = {
                    let f = &self.frames[self.frames.len() - 1];
                    f.locals.get(name).cloned()
                        .or_else(|| f.globals.borrow().get(name).cloned())
                        .or_else(|| f.builtins.get(name).cloned())
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
                let instr_ip = self.frames[fi].ip - 1;  // already incremented
                // Check inline cache first
                if let Some(cached) = self.frames[fi].global_cache.get(instr_ip).and_then(|c| c.clone()) {
                    self.frames[fi].push(cached);
                } else {
                    let name_idx = arg as usize;
                    let name = &self.frames[fi].code.names[name_idx];
                    let val = {
                        let f = &self.frames[self.frames.len() - 1];
                        f.globals.borrow().get(name).cloned()
                            .or_else(|| f.builtins.get(name).cloned())
                    };
                    match val {
                        Some(v) => {
                            // Cache for next time
                            if instr_ip < self.frames[fi].global_cache.len() {
                                self.frames[fi].global_cache[instr_ip] = Some(v.clone());
                            }
                            self.frames[fi].push(v);
                        }
                        None => return Err(PyError::name_error(format!("name '{}' is not defined", name))),
                    }
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
                let idx = arg as usize;
                let (cell_ref, is_freevar, name_str): (Option<PyObjectRef>, bool, String) = {
                    let f = &self.frames[fi];
                    let code = &f.code;
                    if idx < code.cellvars.len() {
                        let name = &code.cellvars[idx];
                        let var_idx = code.varnames.iter().position(|n| n == name)
                            .ok_or_else(|| PyError::name_error(format!("variable '{}' not found", name)))?;
                        (f.fast_locals[var_idx].clone(), false, name.clone())
                    } else {
                        let fv_idx = idx - code.cellvars.len();
                        let name = code.freevars.get(fv_idx)
                            .ok_or_else(|| PyError::runtime_error("freevar index out of range"))?;
                        (f.closure.get(fv_idx).cloned(), true, name.clone())
                    }
                };
                if let Some(cell) = cell_ref {
                    let val = {
                        let obj = cell.borrow();
                        match &*obj {
                            PyObject::Cell { value: Some(inner) } => inner.clone(),
                            PyObject::Cell { value: None } => {
                                return Err(PyError::name_error(format!("variable '{}' referenced before assignment", name_str)));
                            }
                            _ => cell.clone(),
                        }
                    };
                    self.frames[fi].push(val);
                } else if is_freevar {
                    let val = {
                        let globals = self.frames[fi].globals.borrow();
                        globals.get(&name_str).cloned()
                    };
                    if let Some(v) = val {
                        self.frames[fi].push(v);
                    } else {
                        let val = self.frames[fi].builtins.get(&name_str).cloned();
                        if let Some(v) = val {
                            self.frames[fi].push(v);
                        } else {
                            return Err(PyError::name_error(format!("variable '{}' not found", name_str)));
                        }
                    }
                } else {
                    return Err(PyError::name_error(format!("variable '{}' not found", name_str)));
                }
            }

            Opcode::STORE_DEREF => {
                let idx = arg as usize;
                let val = self.frames[fi].pop()?;
                let has_cellvars = idx < self.frames[fi].code.cellvars.len();
                if has_cellvars {
                    let name = &self.frames[fi].code.cellvars[idx];
                    let var_idx = self.frames[fi].code.varnames.iter().position(|n| n == name)
                        .ok_or_else(|| PyError::runtime_error("variable not found"))?;
                    if var_idx < self.frames[fi].fast_locals.len() {
                        if let Some(cell) = self.frames[fi].fast_locals[var_idx].clone() {
                            let mut cell_val = cell.borrow_mut();
                            if let PyObject::Cell { value } = &mut *cell_val {
                                *value = Some(val);
                            }
                        } else {
                            let new_cell = PyObjectRef::new(PyObject::Cell { value: Some(val) });
                            self.frames[fi].fast_locals[var_idx] = Some(new_cell);
                        }
                    } else {
                        let new_cell = PyObjectRef::new(PyObject::Cell { value: Some(val) });
                        self.frames[fi].fast_locals.push(Some(new_cell));
                    }
                } else {
                    let fv_idx = idx - self.frames[fi].code.cellvars.len();
                    if let Some(cell) = self.frames[fi].closure.get(fv_idx).cloned() {
                        let mut cell_val = cell.borrow_mut();
                        if let PyObject::Cell { value } = &mut *cell_val {
                            *value = Some(val);
                        }
                    } else {
                        return Err(PyError::name_error(
                            format!("variable '{}' not found", 
                                self.frames[fi].code.freevars.get(fv_idx).map(|s| s.as_str()).unwrap_or("?"))
                        ));
                    }
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

            // ── Register-based instructions ─────────────────────────
            Opcode::REG_MOV => {
                // Lazily initialize registers
                if self.frames[fi].registers.is_empty() {
                    self.frames[fi].registers = vec![None; 256];
                }
                let dst = (arg >> 4) as usize;
                let src = (arg & 0xF) as usize;
                let val = self.frames[fi].registers[src].clone()
                    .ok_or_else(|| PyError::runtime_error("REG_MOV: source register is empty"))?;
                if dst < self.frames[fi].registers.len() {
                    self.frames[fi].registers[dst] = Some(val);
                }
            }
            Opcode::REG_LOAD_CONST => {
                let dst = (arg >> 4) as usize;
                let const_idx = (arg & 0xFF) as usize;
                let const_val = self.frames[fi].code.consts.get(const_idx).ok_or_else(|| {
                    PyError::runtime_error("REG_LOAD_CONST: index out of range")
                })?.clone();
                let obj = match const_val {
                    ConstValue::None => py_none(),
                    ConstValue::Bool(b) => py_bool(b),
                    ConstValue::Int(s) => {
                        if let Ok(n) = s.parse::<i64>() { py_int(n) }
                        else { let n: BigInt = s.parse().map_err(|_| PyError::value_error("invalid int"))?; PyObjectRef::imm(PyObject::Int(n)) }
                    }
                    ConstValue::Float(s) => py_float(s.parse().map_err(|_| PyError::value_error("invalid float"))?),
                    ConstValue::String(s) => py_str(&s),
                    ConstValue::Bytes(b) => PyObjectRef::imm(PyObject::Bytes(b)),
                    ConstValue::Code(code) => PyObjectRef::imm(PyObject::Code(code)),
                };
                if dst < self.frames[fi].registers.len() {
                    self.frames[fi].registers[dst] = Some(obj);
                }
            }
            Opcode::REG_LOAD_FAST => {
                let dst = (arg >> 4) as usize;
                let var_idx = (arg & 0xFF) as usize;
                let val = self.frames[fi].fast_locals.get(var_idx).and_then(|v| v.clone())
                    .ok_or_else(|| PyError::name_error("local variable referenced before assignment"))?;
                if dst < self.frames[fi].registers.len() {
                    self.frames[fi].registers[dst] = Some(val);
                }
            }
            Opcode::REG_STORE_FAST => {
                let src = (arg >> 4) as usize;
                let var_idx = (arg & 0xFF) as usize;
                let val = self.frames[fi].registers[src].clone()
                    .ok_or_else(|| PyError::runtime_error("REG_STORE_FAST: source register is empty"))?;
                if var_idx < self.frames[fi].fast_locals.len() {
                    self.frames[fi].fast_locals[var_idx] = Some(val.clone());
                }
                let name = self.frames[fi].code.varnames.get(var_idx).ok_or_else(|| {
                    PyError::runtime_error("varname index out of range")
                })?.clone();
                self.frames[fi].locals.insert(name, val);
            }
            Opcode::REG_BINARY_OP => {
                let dst = (arg >> 4) as usize;
                let a_reg = ((arg >> 2) & 0x3) as usize;
                let b_reg = (arg & 0x3) as usize;
                let op = (arg >> 8) as u32;
                let a = self.frames[fi].registers[a_reg].clone()
                    .ok_or_else(|| PyError::runtime_error("REG_BINARY_OP: a is empty"))?;
                let b = self.frames[fi].registers[b_reg].clone()
                    .ok_or_else(|| PyError::runtime_error("REG_BINARY_OP: b is empty"))?;
                let result = match op {
                    0 => py_add(&a, &b),
                    1 => py_sub(&a, &b),
                    2 => py_mul(&a, &b),
                    3 => py_div(&a, &b),
                    4 => py_floor_div(&a, &b),
                    5 => py_mod(&a, &b),
                    6 => py_pow(&a, &b),
                    7 => py_lshift(&a, &b),
                    8 => py_rshift(&a, &b),
                    9 => py_bit_or(&a, &b),
                    10 => py_bit_and(&a, &b),
                    11 => py_bit_xor(&a, &b),
                    13 => py_getitem(&a, &b),
                    _ => return Err(PyError::runtime_error(format!("unknown reg binary op: {}", op))),
                }?;
                if dst < self.frames[fi].registers.len() {
                    self.frames[fi].registers[dst] = Some(result);
                }
            }
            Opcode::REG_LOAD_GLOBAL => {
                let dst = (arg >> 4) as usize;
                let name_idx = (arg & 0xFF) as usize;
                let name = &self.frames[fi].code.names[name_idx];
                // Check inline cache first
                let instr_ip = self.frames[fi].ip - 1;
                if let Some(cached) = self.frames[fi].global_cache.get(instr_ip).and_then(|c| c.clone()) {
                    if dst < self.frames[fi].registers.len() {
                        self.frames[fi].registers[dst] = Some(cached);
                    }
                } else {
                    let val = self.frames[fi].globals.borrow().get(name).cloned()
                        .or_else(|| self.frames[fi].builtins.get(name).cloned());
                    if let Some(v) = val {
                        if instr_ip < self.frames[fi].global_cache.len() {
                            self.frames[fi].global_cache[instr_ip] = Some(v.clone());
                        }
                        if dst < self.frames[fi].registers.len() {
                            self.frames[fi].registers[dst] = Some(v);
                        }
                    } else {
                        return Err(PyError::name_error(format!("name '{}' is not defined", name)));
                    }
                }
            }
            Opcode::REG_RETURN => {
                let src = (arg & 0xFF) as usize;
                let val = self.frames[fi].registers[src].clone()
                    .ok_or_else(|| PyError::runtime_error("REG_RETURN: register is empty"))?;
                return Ok(Some(val));
            }
            Opcode::REG_BUILD_LIST => {
                // arg: upper 4 bits = dst, lower 4 bits = count
                let dst = (arg >> 4) as usize;
                let count = (arg & 0xF) as usize;
                let mut items = Vec::with_capacity(count);
                for i in 0..count {
                    if let Some(val) = self.frames[fi].registers[i].clone() {
                        items.push(val);
                    }
                }
                if dst < self.frames[fi].registers.len() {
                    self.frames[fi].registers[dst] = Some(py_list(items));
                }
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

            Opcode::MAKE_CELL => {
                let idx = arg as usize;
                let frame = &mut self.frames[fi];
                if idx < frame.fast_locals.len() {
                    let val = frame.fast_locals[idx].take();
                    let cell = PyObjectRef::new(PyObject::Cell { value: val });
                    frame.fast_locals[idx] = Some(cell);
                }
            }

            Opcode::COPY_FREE_VARS => {
                let nfree = arg as usize;
                let mut cells = Vec::with_capacity(nfree);
                for _ in 0..nfree {
                    cells.push(self.frames[fi].pop()?);
                }
                // Store the closure tuple on the stack for MAKE_FUNCTION to consume
                self.frames[fi].push(PyObjectRef::imm(PyObject::Tuple(cells)));
            }

            Opcode::MAKE_FUNCTION => {
                let has_closure = (arg & 0x100) != 0;
                let n_defaults = (arg & 0xFF) as usize;
                let mut defaults = Vec::new();
                for _ in 0..n_defaults {
                    defaults.push(self.frames[fi].pop()?);
                }
                defaults.reverse();
                let code_obj = self.frames[fi].pop()?;
                let code = match &*code_obj.borrow() {
                    PyObject::Code(c) => c.as_ref().clone(),
                    _ => return Err(PyError::runtime_error("MAKE_FUNCTION: expected code object")),
                };
                let closure = if has_closure {
                    let closure_tuple = self.frames[fi].pop()?;
                    let items = closure_tuple.borrow();
                    if let PyObject::Tuple(items) = &*items {
                        items.clone()
                    } else {
                        Vec::new()
                    }
                } else {
                    Vec::new()
                };
                let name = if !code.name.is_empty() {
                    code.name.clone()
                } else {
                    "<function>".to_string()
                };
                let globals = self.frames[fi].globals.clone();
                let func = PyObjectRef::new(PyObject::Function {
                    code,
                    globals,
                    name,
                    defaults,
                    closure,
                    dict: HashMap::new(),
                    jit_ptr: std::cell::Cell::new(0),
                    jit_consts: std::cell::RefCell::new(Vec::new()),
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
                self.frames[fi].push(PyObjectRef::new(PyObject::Set(PySet::from_vec(items)?)));
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
                self.frames[fi].push(PyObjectRef::imm(PyObject::Slice {
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
                     12 => {
                         (|| -> PyResult<PyObjectRef> {
                             if let Some(r) = crate::object::try_dunder_binop(&left, &right, "__matmul__")? {
                                 return Ok(r);
                             }
                             if let Some(r) = crate::object::try_dunder_binop(&right, &left, "__rmatmul__")? {
                                 return Ok(r);
                             }
                             Err(PyError::type_error(format!("unsupported operand type(s) for @: '{}' and '{}'",
                                 left.borrow().type_name(), right.borrow().type_name())))
                         })()
                     }
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
                // Check for user-class instance (needs __iter__ protocol)
                let is_instance = val.borrow().type_name() == "instance";
                if is_instance {
                    use crate::object::ObjectAccess;
                    let raw_method = val.borrow().get_attribute("__iter__")
                        .map_err(|_| PyError::type_error(format!("'{}' object is not iterable", val.borrow().type_name())))?;
                    let val_clone = val.clone();
                    let iter_method = PyObjectRef::imm(PyObject::BoundMethod {
                        func: raw_method,
                        self_obj: val_clone,
                    });
                    let iterator = self.call_function(iter_method, vec![], vec![])?;
                    // Eagerly consume via BoundMethod wrapping (binds self properly)
                    let mut items: Vec<PyObjectRef> = Vec::new();
                    loop {
                        let next_result = {
                            let next_attr = iterator.borrow().get_attribute("__next__");
                            match next_attr {
                                Ok(f) => {
                                    // Need to wrap as BoundMethod for self-binding
                                    let obj_clone = iterator.clone();
                                    let bound = PyObjectRef::imm(PyObject::BoundMethod {
                                        func: f.clone(),
                                        self_obj: obj_clone,
                                    });
                                    self.call_function(bound, vec![], vec![])
                                }
                                Err(_) => break,
                            }
                        };
                        match next_result {
                            Ok(val) => items.push(val),
                            Err(PyError::StopIteration) => break,
                            Err(e) => return Err(e),
                        }
                    }
                    self.frames[fi].push(PyObjectRef::new(PyObject::ListIter { list: items, index: 0 }));
                } else {
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
                        self.frames[fi].push(PyObjectRef::new(PyObject::ListIter { list: s.to_vec(), index: 0 }));
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
                        let fixed = PyObjectRef::imm(PyObject::BuiltinMethod {
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
                        _ => {
                            // Not a built-in iterator — check for __next__ protocol
                            if obj.type_name() == "instance" {
                                return self.for_iter_next(iter_val.clone(), arg);
                            }
                            return Err(PyError::type_error("for_iter on non-iterable"))
                        },
                    }
                };
                if is_exhausted {
                    self.frames[fi].ip = arg as usize;
                } else {
                    let val = self.frames[fi].pop()?;
                    let item = {
                        // Convert plain List to ListIter for O(1) iteration
                        let is_plain_list = matches!(&*val.borrow(), PyObject::List(..));
                        if is_plain_list {
                            let list_clone = {
                                let obj = val.borrow();
                                if let PyObject::List(v) = &*obj { v.clone() } else { unreachable!() }
                            };
                            *val.borrow_mut() = PyObject::ListIter { list: list_clone, index: 0 };
                        }
                        let mut obj = val.borrow_mut();
                        match &mut *obj {
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
                                                let is_instance_obj = matches!(&*obj.borrow(), PyObject::Instance { .. });
                                                if is_instance_obj {
                                                    return Some(PyObjectRef::imm(PyObject::BoundMethod {
                                                        func: val.clone(),
                                                        self_obj: obj.clone(),
                                                    }));
                                                } else {
                                                    return Some(val.clone());
                                                }
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
                            let type_name = obj_borrowed.type_name();
                            // Check inline cache first
                            let cached = ATTR_CACHE.with(|c| c.borrow().get(&(type_name.clone(), name.clone())).copied());
                            if let Some(func) = cached {
                                drop(obj_borrowed);
                                Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                                    name: name.clone(),
                                    func,
                                    self_obj: obj.clone(),
                                }))
                            } else {
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
                                    // Cache for next time
                                    ATTR_CACHE.with(|c| { c.borrow_mut().insert((type_name.clone(), n.clone()), func); });
                                    Ok(PyObjectRef::imm(PyObject::BuiltinMethod {
                                        name: n,
                                        func,
                                        self_obj: obj.clone(),
                                    }))
                                } else if is_function {
                                    Ok(attr)
                                } else {
                                    Ok(attr)
                                }
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
                // Check for __set__ descriptor protocol on Instance types
                let descriptor_clone = {
                    let obj_borrowed = obj.borrow();
                    if let PyObject::Instance { typ, .. } = &*obj_borrowed {
                        let typ_ref = typ.borrow();
                        if let PyObject::Type { dict: type_dict, .. } = &*typ_ref {
                            type_dict.get(&name).cloned()
                        } else { None }
                    } else { None }
                };
                if let Some(descriptor) = descriptor_clone {
                    let setter_method = {
                        descriptor.borrow().get_attribute("__set__").ok()
                    };
                    if let Some(setter_method) = setter_method {
                        let (setter_func, setter_self) = {
                            let b = setter_method.borrow();
                            match &*b {
                                PyObject::BuiltinMethod { func, self_obj, .. } => (func.clone(), self_obj.clone()),
                                _ => return Err(PyError::runtime_error("expected __set__ method")),
                            }
                        };
                        setter_func(&[setter_self, descriptor, obj.clone(), val])?;
                        return Ok(None);
                    } else {
                        // Descriptor exists but no __set__ (non-data descriptor), fall through
                    }
                }
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
                    v.add(val)?;
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
                    d.set(key, val)?;
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
                // Restore stack to the depth before the handler was set up
                if let Some(handler) = self.frames[fi].exception_handlers.pop() {
                    self.frames[fi].stack.truncate(handler.stack_depth);
                }
            }

            Opcode::PUSH_EXC_INFO => {
                // No-op on value stack — exc_obj stays on top for DUP_TOP below
            }

            Opcode::POP_EXCEPT => {
                // No-op to match PUSH_EXC_INFO — balance the pair
            }

            Opcode::GET_AITER => {
                // async for: call __aiter__ on the top of stack
                let obj = self.frames[fi].peek(0)?;
                let aiter_method = obj.borrow().get_attribute("__aiter__")
                    .map_err(|_| PyError::type_error("object does not support async iteration"))?;
                let result = self.call_function(aiter_method, vec![], vec![])?;
                self.frames[fi].pop();
                self.frames[fi].push(result);
            }

            Opcode::GET_ANEXT => {
                // async for: get __anext__ method from the async iterator
                let obj = self.frames[fi].peek(0)?;
                let anext_method = obj.borrow().get_attribute("__anext__")
                    .map_err(|_| PyError::type_error("async iterator has no __anext__"))?;
                self.frames[fi].pop();
                self.frames[fi].push(anext_method);
            }

            Opcode::END_FOR => {
                // Pop the iterator/async-iterator after a for loop
                let _ = self.frames[fi].pop();
            }

            Opcode::BEFORE_ASYNC_WITH => {
                // async with: call __aenter__ and push __aexit__ for later
                let mgr = self.frames[fi].pop()?;
                let aenter_method = mgr.borrow().get_attribute("__aenter__")
                    .map_err(|_| PyError::attribute_error("async context manager has no __aenter__"))?;
                let result = self.call_function(aenter_method, vec![], vec![])?;
                self.frames[fi].push(mgr);
                self.frames[fi].push(result);
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
                match self.frames[fi].pop() {
                    Ok(exc) => {
                        return Err(PyError::Exception(format!("re-raise"), exc));
                    }
                    Err(_) => return Err(PyError::runtime_error("re-raise")),
                }
            }

            Opcode::RAISE_VARARGS => {
                let nargs = arg;
                match nargs {
                    0 => return Err(PyError::runtime_error("re-raise")),
                    1 => {
                        let exc = self.frames[fi].pop()?;
                        // If the raised value is a callable (class/factory), call it first
                        let is_callable = !matches!(&*exc.borrow(), PyObject::Str(_) | PyObject::Exception { .. });
                        let exc = if is_callable {
                            let exc_clone = exc.clone();
                            match self.call_function(exc_clone, vec![], vec![]) {
                                Ok(instance) => instance,
                                Err(_) => return Err(PyError::type_error("exceptions must be str or Exception instances")),
                            }
                        } else {
                            exc
                        };
                        let msg = match &*exc.borrow() {
                            PyObject::Str(s) => s.clone(),
                            PyObject::Exception { args, .. } => {
                                if !args.is_empty() { args[0].str() } else { "".to_string() }
                            }
                            _ => return Err(PyError::type_error("exceptions must be str or Exception instances")),
                        };
                        // raise StopIteration → PyError::StopIteration (needed by for_iter/next protocol)
                        if msg.is_empty() {
                            let exc_borrowed = exc.borrow();
                            let is_stop = match &*exc_borrowed {
                                PyObject::Exception { ref typ, .. } if typ == "StopIteration" => true,
                                PyObject::Type { name, .. } if name == "StopIteration" => true,
                                _ => false,
                            };
                            if is_stop {
                                return Err(PyError::StopIteration);
                            }
                        }
                        return Err(PyError::Exception(msg, exc));
                    }
                    2 => {
                        let cause = self.frames[fi].pop()?;
                        let exc = self.frames[fi].pop()?;
                        let exc_msg = match &*exc.borrow() {
                            PyObject::Exception { args, .. } => {
                                if !args.is_empty() { args[0].str() } else { exc.str() }
                            }
                            _ => exc.str(),
                        };
                        let cause_msg = match &*cause.borrow() {
                            PyObject::Exception { args, .. } => {
                                if !args.is_empty() { args[0].str() } else { cause.str() }
                            }
                            _ => cause.str(),
                        };
                        // Set __cause__ on the exception object (cause field changed to PyObjectRef)
                        if let PyObject::Exception { cause: ref mut cause_field, .. } = &mut *exc.borrow_mut() {
                            *cause_field = Some(cause.clone());
                        }
                        return Err(PyError::Exception(format!("{} (caused by: {})", exc_msg, cause_msg), exc));
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
                if let Some(module) = self.modules.get(&name) {
                    self.frames[fi].push(module.clone());
                } else if let Ok(module) = self.import_module_from_file(&name) {
                    self.modules.insert(name.clone(), module.clone());
                    // Also add to sys.modules if available
                    if let Some(sys_mod) = self.modules.get("sys") {
                        if let PyObject::Module { dict, .. } = &*sys_mod.borrow() {
                            if let Some(mod_dict) = dict.get("modules") {
                                mod_dict.borrow_mut().set_attribute(&name, module.clone()).ok();
                            }
                        }
                    }
                    self.frames[fi].push(module);
                } else {
                    return Err(PyError::ImportError(format!("No module named '{}'", name)));
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
                            return Err(PyError::ImportError(format!("cannot import name '{}' from '{}'", name,
                                module.borrow().type_name())));
                        }
                    }
                    _ => return Err(PyError::runtime_error("IMPORT_FROM on non-module")),
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
                        if let Some(var_idx) = f.code.varnames.iter().position(|n| n == name) {
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
                // Look up __enter__ and call it, keeping manager on stack
                let mgr = self.frames[fi].peek(0)?;
                let _exit_method = mgr.borrow().get_attribute("__exit__").ok();
                let enter_raw = mgr.borrow().get_attribute("__enter__").ok();
                if let Some(enter_raw) = enter_raw {
                    let is_builtin = matches!(&*enter_raw.borrow(), PyObject::BuiltinMethod { .. });
                    let enter = if is_builtin {
                        let b = enter_raw.borrow();
                        match &*b {
                            PyObject::BuiltinMethod { name, func, .. } => {
                                PyObjectRef::imm(PyObject::BuiltinMethod {
                                    name: name.clone(),
                                    func: *func,
                                    self_obj: mgr.clone(),
                                })
                            }
                            _ => unreachable!(),
                        }
                    } else {
                        PyObjectRef::imm(PyObject::BoundMethod {
                            func: enter_raw,
                            self_obj: mgr.clone(),
                        })
                    };
                    let result = self.call_function(enter, vec![], vec![])?;
                    self.frames[fi].push(result);
                } else {
                    self.frames[fi].push(py_none());
                }
            }

            Opcode::WITH_EXIT => {
                // Stack: [..., exception_obj, manager]
                // Call manager.__exit__(exc_type, exc_val, traceback)
                let mgr = self.frames[fi].pop()?;
                let (typ_str, val) = {
                    let exc = self.frames[fi].peek(0)?;
                    let exc_borrowed = exc.borrow();
                    match &*exc_borrowed {
                        PyObject::Exception { typ, args, .. } => {
                            (py_str(typ), args.first().cloned().unwrap_or_else(|| py_none()))
                        }
                        _ => (py_str("Exception"), py_none()),
                    }
                };
                let exit_method = mgr.borrow().get_attribute("__exit__")
                    .map_err(|_| PyError::attribute_error("context manager has no __exit__"))?;
                // Bind the manager as self so __exit__ can access it
                let bound = PyObjectRef::imm(PyObject::BoundMethod {
                    func: exit_method,
                    self_obj: mgr,
                });
                let result = self.call_function(bound, vec![typ_str, val, py_none()], vec![])?;
                self.frames[fi].push(result);
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

        if let PyObject::Partial { func, args: partial_args } = &*callable.borrow() {
            let func = func.clone();
            let mut all_args = partial_args.clone();
            all_args.extend(args);
            return self.call_function(func, all_args, keywords);
        }

        if let PyObject::Function { code, globals: func_globals, defaults, jit_ptr, closure, jit_consts, .. } = &*callable.borrow() {
            // Try JIT execution: if we have compiled code, use it directly
            let jit_fn: usize = jit_ptr.get();
            if jit_fn != 0 {
                let func: extern "C" fn(*const PyObjectRef, usize, *const PyObjectRef, *mut PyObjectRef) = 
                    unsafe { std::mem::transmute(jit_fn) };
                let mut const_cache = jit_consts.borrow_mut();
                if const_cache.is_empty() {
                    *const_cache = crate::jit::JitCompiler::precompute_consts(code);
                }
                let mut result = crate::object::py_none();
                func(args.as_ptr(), args.len(), const_cache.as_ptr(), &mut result as *mut PyObjectRef);
                return Ok(result);
            }
            // Try simple execution without Frame creation
            if defaults.is_empty() && keywords.is_empty() {
                if let Some(result) = Self::try_exec_simple(code, &args) {
                    return result;
                }
            }
            // Try JIT compilation on first call
            let mut jit = self.jit.borrow_mut();
            if let Some(compiled) = jit.compile(code) {
                let fn_ptr = compiled as *const () as usize;
                drop(jit);
                jit_ptr.set(fn_ptr);
                let jit_consts_compiled = crate::jit::JitCompiler::precompute_consts(code);
                let mut result = crate::object::py_none();
                compiled(args.as_ptr(), args.len(), jit_consts_compiled.as_ptr(), &mut result as *mut PyObjectRef);
                let mut const_cache = jit_consts.borrow_mut();
                *const_cache = jit_consts_compiled;
                return Ok(result);
            }
            drop(jit);
            let func_globals = func_globals.clone();
            let defaults = defaults.clone();
            let code_rc = Rc::new(code.clone());
            let mut new_frame = Frame::new(Rc::clone(&code_rc), func_globals, Rc::clone(&self.builtins));
            new_frame.closure = closure.clone();
            let code = code;

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
                    let default_idx = num_defaults.saturating_sub(named_params - i);
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
                            dict.set(py_str(key), value.clone())?;
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
                        let mut new_frame = Frame::new(Rc::new(code), func_globals, Rc::clone(&self.builtins));
                        // Set self at index 0
                        if !new_frame.code.varnames.is_empty() {
                            new_frame.fast_locals[0] = Some(instance.clone());
                            new_frame.locals.insert(new_frame.code.varnames[0].clone(), instance.clone());
                        }
                        for (i, arg_name) in new_frame.code.varnames.iter().enumerate().skip(1) {
                            if i - 1 < args.len() {
                                new_frame.fast_locals[i] = Some(args[i - 1].clone());
                                new_frame.locals.insert(arg_name.clone(), args[i - 1].clone());
                            } else {
                                new_frame.fast_locals[i] = Some(py_none());
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
                    let new_frame = Frame::new(Rc::new(code), namespace.clone(), Rc::clone(&self.builtins));
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
                dict: namespace_dict.clone(),
                bases: bases_vec.clone(),
                mro: vec![],
            });

            let mut mro = vec![class.clone()];
            // C3 linearization for proper method resolution
            mro.extend(c3_linearize(&bases_vec));
            if let PyObject::Type { mro: mro_field, .. } = &mut *class.borrow_mut() {
                *mro_field = mro;
            }

            // __set_name__ protocol: for each descriptor in the class dict that has __set_name__, call it
            for (attr_name, value) in namespace_dict.iter() {
                let has_set_name = value.borrow().get_attribute("__set_name__").is_ok();
                if has_set_name {
                    let set_name_method = value.borrow().get_attribute("__set_name__").unwrap();
                    let _ = self.call_function(set_name_method, vec![class.clone(), py_str(attr_name)], vec![]);
                }
            }

            // __init_subclass__ protocol: call on each base class
            for base in &bases_vec {
                if let Ok(init_subclass) = base.borrow().get_attribute("__init_subclass__") {
                    let _ = self.call_function(init_subclass, vec![class.clone()], vec![]);
                }
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
                    let (typ, cause) = match error {
                        PyError::TypeError(_) => ("TypeError".to_string(), None),
                        PyError::ValueError(_) => ("ValueError".to_string(), None),
                        PyError::NameError(_) => ("NameError".to_string(), None),
                        PyError::AttributeError(_) => ("AttributeError".to_string(), None),
                        PyError::IndexError(_) => ("IndexError".to_string(), None),
                        PyError::KeyError(_) => ("KeyError".to_string(), None),
                        PyError::ZeroDivisionError(_) => ("ZeroDivisionError".to_string(), None),
                        PyError::RuntimeError(_) => ("RuntimeError".to_string(), None),
                        PyError::StopIteration => ("StopIteration".to_string(), None),
                        PyError::AssertionError(_) => ("AssertionError".to_string(), None),
                        PyError::ImportError(_) => ("ImportError".to_string(), None),
                        PyError::Exception(_, exc) => {
                            let exc_borrow = exc.borrow();
                            match &*exc_borrow {
                                PyObject::Exception { typ, cause, .. } => (typ.clone(), cause.clone()),
                                _ => ("Exception".to_string(), None),
                            }
                        }
                        _ => ("Exception".to_string(), None),
                    };
                    PyObjectRef::imm(PyObject::Exception {
                        typ,
                        args: vec![py_str(&error.message())],
                        cause,
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

impl VirtualMachine {
    /// Call __next__ on a user-class iterator. Used by FOR_ITER for Instance types.
    fn for_iter_next(&mut self, iter_val: PyObjectRef, jump_offset: u32) -> PyResult<Option<PyObjectRef>> {
        use crate::object::ObjectAccess;
        let next_method = iter_val.borrow().get_attribute("__next__");
        if let Ok(func) = next_method {
            match self.call_function(func, vec![], vec![]) {
                Ok(val) => {
                    self.frames.last_mut().unwrap().push(iter_val);
                    self.frames.last_mut().unwrap().push(val);
                    Ok(None)
                }
                Err(e) if matches!(&e, PyError::StopIteration) => {
                    self.frames.last_mut().unwrap().ip = jump_offset as usize;
                    Ok(None)
                }
                Err(e) => Err(e),
            }
        } else {
            self.frames.last_mut().unwrap().ip = jump_offset as usize;
            Ok(None)
        }
    }
}
