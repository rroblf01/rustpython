use std::collections::{HashMap, HashSet};
use crate::ast::*;
use crate::bytecode::*;

pub struct Compiler {
    code: CodeObject,
    labels: Vec<Vec<usize>>,
    label_positions: Vec<usize>,
    label_stack: Vec<Vec<(usize, u32)>>,
    loop_stack: Vec<LoopInfo>,
    scope: ScopeType,
    global_names: HashSet<String>,
    nonlocal_names: HashSet<String>,
    free_vars: HashSet<String>,
    cell_vars: HashSet<String>,
    scope_stack: Vec<ScopeInfo>,
}

struct LoopInfo {
    start_label: usize,
    end_label: usize,
}

struct ScopeInfo {
    scope: ScopeType,
    global_names: HashSet<String>,
    nonlocal_names: HashSet<String>,
    varnames: Vec<String>,
    cellvars: Vec<String>,
    freevars: Vec<String>,
}

#[derive(Clone, PartialEq)]
enum ScopeType {
    Module,
    Function,
    Class,
    ClassBody,
    Comprehension,
}

impl Compiler {
    pub fn new() -> Self {
        Compiler {
            code: CodeObject::new("<module>".to_string()),
            labels: Vec::new(),
            label_positions: Vec::new(),
            label_stack: Vec::new(),
            loop_stack: Vec::new(),
            scope: ScopeType::Module,
            global_names: HashSet::new(),
            nonlocal_names: HashSet::new(),
            free_vars: HashSet::new(),
            cell_vars: HashSet::new(),
            scope_stack: Vec::new(),
        }
    }

    pub fn compile(&mut self, program: &Program, filename: &str) -> Result<CodeObject, String> {
        self.code.filename = filename.to_string();
        // Ensure constant 0 is always None for module return
        if self.code.consts.is_empty() || !matches!(&self.code.consts[0], ConstValue::None) {
            self.code.consts.insert(0, ConstValue::None);
        }
        match program {
            Program::Module(stmts) => {
                self.compile_stmts(stmts)?;
            }
            Program::Expression(expr) => {
                self.compile_expr(expr)?;
                self.emit(Opcode::RETURN_VALUE, 0);
            }
        }
        if self.scope == ScopeType::Module {
            self.emit(Opcode::LOAD_CONST, 0);
            self.emit(Opcode::RETURN_VALUE, 0);
        }
        Ok(self.code.clone())
    }

    fn enter_scope(&mut self, scope: ScopeType) {
        let info = ScopeInfo {
            scope: self.scope.clone(),
            global_names: std::mem::take(&mut self.global_names),
            nonlocal_names: std::mem::take(&mut self.nonlocal_names),
            varnames: std::mem::take(&mut self.code.varnames),
            cellvars: std::mem::take(&mut self.code.cellvars),
            freevars: std::mem::take(&mut self.code.freevars),
        };
        self.scope_stack.push(info);
        self.scope = scope;
    }

    fn leave_scope(&mut self) {
        if let Some(info) = self.scope_stack.pop() {
            self.scope = info.scope;
            self.global_names = info.global_names;
            self.nonlocal_names = info.nonlocal_names;
            self.code.varnames = info.varnames;
            self.code.cellvars = info.cellvars;
            self.code.freevars = info.freevars;
        }
    }

    fn get_var_index(&mut self, name: &str) -> Option<usize> {
        self.code.varnames.iter().position(|n| n == name)
    }

    fn add_varname(&mut self, name: &str) -> usize {
        if let Some(idx) = self.get_var_index(name) {
            return idx;
        }
        self.code.varnames.push(name.to_string());
        self.code.varnames.len() - 1
    }

    fn get_name_index(&mut self, name: &str) -> usize {
        if let Some(idx) = self.code.names.iter().position(|n| n == name) {
            return idx;
        }
        self.code.names.push(name.to_string());
        self.code.names.len() - 1
    }

    fn get_const_index(&mut self, c: ConstValue) -> usize {
        if let Some(idx) = self.code.consts.iter().position(|x| {
            match (x, &c) {
                (ConstValue::None, ConstValue::None) => true,
                (ConstValue::Bool(a), ConstValue::Bool(b)) => a == b,
                (ConstValue::Int(a), ConstValue::Int(b)) => a == b,
                (ConstValue::Float(a), ConstValue::Float(b)) => a == b,
                (ConstValue::String(a), ConstValue::String(b)) => a == b,
                _ => false,
            }
        }) {
            return idx;
        }
        self.code.consts.push(c);
        self.code.consts.len() - 1
    }

    fn emit(&mut self, op: Opcode, arg: u32) {
        self.code.instructions.push(Instr::with_arg(op, arg));
    }

    fn new_label(&mut self) -> usize {
        self.labels.push(Vec::new());
        self.label_positions.push(0);
        self.labels.len() - 1
    }

    fn fix_label(&mut self, label: usize) {
        let pos = self.code.instructions.len();
        for &instr_pos in &self.labels[label] {
            let offset = pos as u32 - self.code.instructions[instr_pos].arg;
            self.code.instructions[instr_pos].arg = offset;
        }
        self.labels[label].clear();
    }

    fn mark_label(&mut self, label: usize) {
        self.label_positions[label] = self.code.instructions.len();
    }

    fn emit_jump(&mut self, op: Opcode, label: usize) {
        self.code.instructions.push(Instr::with_arg(op, 0));
        self.labels[label].push(self.code.instructions.len() - 1);
    }

    fn emit_backward_jump(&mut self, target_label: usize) {
        let target = self.label_positions[target_label];
        let jump_pos = self.code.instructions.len();
        let offset = (jump_pos as u32).wrapping_sub(target as u32);
        self.emit(Opcode::JUMP_BACKWARD, offset);
    }

    // ---- Statement compilation ----

    fn compile_stmts(&mut self, stmts: &[Stmt]) -> Result<(), String> {
        for stmt in stmts {
            self.compile_stmt(stmt)?;
        }
        Ok(())
    }

    fn compile_stmt(&mut self, stmt: &Stmt) -> Result<(), String> {
        match stmt {
            Stmt::Expr(expr) => {
                self.compile_expr(expr)?;
                self.emit(Opcode::POP_TOP, 0);
            }
            Stmt::Pass => {}
            Stmt::Break => {
                if let Some(loop_info) = self.loop_stack.last() {
                    self.emit_jump(Opcode::JUMP, loop_info.end_label);
                } else {
                    return Err("'break' outside loop".to_string());
                }
            }
            Stmt::Continue => {
                if let Some(loop_info) = self.loop_stack.last() {
                    self.emit_backward_jump(loop_info.start_label);
                } else {
                    return Err("'continue' outside loop".to_string());
                }
            }
            Stmt::Return(value) => {
                if self.scope == ScopeType::Module {
                    if let Some(expr) = value {
                        self.compile_expr(expr)?;
                    } else {
                        let const_idx = self.get_const_index(ConstValue::None) as u32;
                        self.emit(Opcode::LOAD_CONST, const_idx);
                    }
                } else {
                    if let Some(expr) = value {
                        self.compile_expr(expr)?;
                    } else {
                        let const_idx = self.get_const_index(ConstValue::None) as u32;
                        self.emit(Opcode::LOAD_CONST, const_idx);
                    }
                }
                self.emit(Opcode::RETURN_VALUE, 0);
            }
            Stmt::Assign { targets, value } => {
                if targets.len() == 1 {
                    self.compile_expr(value)?;
                    self.compile_assign_target(&targets[0])?;
                } else {
                    // Multiple targets: a = b = c
                    self.compile_expr(value)?;
                    for target in targets {
                        self.emit(Opcode::COPY, 1);
                        self.compile_assign_target(target)?;
                    }
                    self.emit(Opcode::POP_TOP, 0);
                }
            }
            Stmt::AugAssign { target, op, value } => {
                self.compile_expr(target)?;
                self.compile_expr(value)?;
                let bin_op = match op {
                    Operator::Add => 0,  // BINARY_OP + 
                    Operator::Sub => 1,
                    Operator::Mult => 2,
                    Operator::Div => 3,
                    Operator::FloorDiv => 4,
                    Operator::Mod => 5,
                    Operator::Pow => 6,
                    Operator::LShift => 7,
                    Operator::RShift => 8,
                    Operator::BitOr => 9,
                    Operator::BitXor => 10,
                    Operator::BitAnd => 11,
                    Operator::MatMult => 12,
                };
                self.emit(Opcode::BINARY_OP, bin_op);
                self.compile_assign_target(target)?;
            }
            Stmt::If { test, body, orelse } => {
                self.compile_expr(test)?;
                let else_label = self.new_label();
                self.emit_jump(Opcode::POP_JUMP_IF_FALSE, else_label);
                self.compile_stmts(body)?;
                if !orelse.is_empty() {
                    let end_label = self.new_label();
                    self.emit_jump(Opcode::JUMP, end_label);
                    self.fix_label(else_label);
                    self.compile_stmts(orelse)?;
                    self.fix_label(end_label);
                } else {
                    self.fix_label(else_label);
                }
            }
            Stmt::While { test, body, orelse } => {
                let start_label = self.new_label();
                let else_label = self.new_label();
                let end_label = self.new_label();
                self.mark_label(start_label);
                self.loop_stack.push(LoopInfo { start_label, end_label });
                self.compile_expr(test)?;
                self.emit_jump(Opcode::POP_JUMP_IF_FALSE, else_label);
                self.compile_stmts(body)?;
                self.emit_backward_jump(start_label);
                self.fix_label(else_label);
                if !orelse.is_empty() {
                    self.compile_stmts(orelse)?;
                }
                self.fix_label(end_label);
                self.loop_stack.pop();
            }
            Stmt::For { target, iter, body, orelse } => {
                self.compile_expr(iter)?;
                self.emit(Opcode::GET_ITER, 0);
                let start_label = self.new_label();
                let else_label = self.new_label();
                let end_label = self.new_label();
                self.loop_stack.push(LoopInfo { start_label, end_label });
                self.mark_label(start_label);
                self.emit_jump(Opcode::FOR_ITER, else_label);
                self.compile_assign_target(target)?;
                self.compile_stmts(body)?;
                self.emit_backward_jump(start_label);
                self.fix_label(else_label);
                if !orelse.is_empty() {
                    self.compile_stmts(orelse)?;
                }
                self.emit(Opcode::POP_ITER, 0);
                self.fix_label(end_label);
                self.loop_stack.pop();
            }
            Stmt::FunctionDef { name, args, body, decorator_list, returns: _ } => {
                for decorator in decorator_list {
                    self.compile_expr(decorator)?;
                }

                self.compile_function(name.clone(), args, body)?;

                for _ in decorator_list {
                    self.emit(Opcode::CALL, 1);
                }
                let name_idx = self.get_name_index(name) as u32;
                self.emit(Opcode::STORE_NAME, name_idx);
            }
            Stmt::ClassDef { name, bases, keywords: kw, body, decorator_list: _ } => {
                // Extract docstring from first statement if present
                let docstring = body.first().and_then(|s| {
                    if let Stmt::Expr(expr) = s {
                        if let Expr::Constant(Constant::String(doc)) = expr.as_ref() {
                            Some(doc.clone())
                        } else { None }
                    } else { None }
                });
                self.emit(Opcode::LOAD_BUILD_CLASS, 0);
                self.compile_class_body(name.clone(), body)?;
                let class_name_idx = self.get_const_index(ConstValue::String(name.clone())) as u32;
                self.emit(Opcode::LOAD_CONST, class_name_idx);
                if bases.is_empty() {
                    let const_none = self.get_const_index(ConstValue::None) as u32;
                    self.emit(Opcode::LOAD_CONST, const_none);
                } else if bases.len() == 1 {
                    self.compile_expr(&bases[0])?;
                } else {
                    for base in bases {
                        self.compile_expr(base)?;
                    }
                    self.emit(Opcode::BUILD_TUPLE, bases.len() as u32);
                }
                let kw_count = kw.len() as u32;
                for k in kw {
                    self.compile_expr(&k.value)?;
                }
                self.emit(Opcode::CALL, 3 + kw_count);
                // Set __doc__ on class if present
                if let Some(doc) = docstring {
                    self.emit(Opcode::DUP_TOP, 0);
                    let doc_idx = self.get_const_index(ConstValue::String(doc)) as u32;
                    self.emit(Opcode::LOAD_CONST, doc_idx);
                    let doc_attr_idx = self.get_name_index("__doc__") as u32;
                    self.emit(Opcode::STORE_ATTR, doc_attr_idx);
                }
                let name_idx = self.get_name_index(name) as u32;
                self.emit(Opcode::STORE_NAME, name_idx);
            }
            Stmt::Import(names) => {
                for alias in names {
                    let name_idx = self.get_name_index(&alias.name) as u32;
                    let const_none = self.get_const_index(ConstValue::None) as u32;
                    self.emit(Opcode::LOAD_CONST, const_none);
                    self.emit(Opcode::LOAD_CONST, const_none);
                    self.emit(Opcode::LOAD_CONST, const_none);
                    self.emit(Opcode::IMPORT_NAME, name_idx);
                    if let Some(asname) = &alias.asname {
                        let store_idx = self.get_name_index(asname) as u32;
                        self.emit(Opcode::STORE_NAME, store_idx);
                    } else {
                        let dot_pos = alias.name.find('.');
                        if let Some(pos) = dot_pos {
                            let first_name = &alias.name[..pos];
                            let name_idx = self.get_name_index(first_name) as u32;
                            self.emit(Opcode::STORE_NAME, name_idx);
                        } else {
                            self.emit(Opcode::STORE_NAME, name_idx);
                        }
                    }
                }
            }
            Stmt::ImportFrom { module, names, level: _ } => {
                let module_name = module.clone().unwrap_or_default();
                let name_idx = self.get_name_index(&module_name) as u32;
                let const_none = self.get_const_index(ConstValue::None) as u32;
                self.emit(Opcode::LOAD_CONST, const_none);
                self.emit(Opcode::LOAD_CONST, const_none);
                self.emit(Opcode::LOAD_CONST, const_none);
                self.emit(Opcode::IMPORT_NAME, name_idx);
                for alias in names {
                    let import_name_idx = self.get_name_index(&alias.name) as u32;
                    self.emit(Opcode::IMPORT_FROM, import_name_idx);
                    if let Some(asname) = &alias.asname {
                        let store_idx = self.get_name_index(asname) as u32;
                        self.emit(Opcode::STORE_NAME, store_idx);
                    } else {
                        let store_idx = self.get_name_index(&alias.name) as u32;
                        self.emit(Opcode::STORE_NAME, store_idx);
                    }
                }
            }
            Stmt::Global(names) => {
                for name in names {
                    self.global_names.insert(name.clone());
                }
            }
            Stmt::Nonlocal(names) => {
                for name in names {
                    self.nonlocal_names.insert(name.clone());
                }
            }
            Stmt::Delete(targets) => {
                for target in targets {
                    match target {
                        Expr::Name(name) => {
                            if self.scope == ScopeType::Module {
                                let idx = self.get_name_index(name) as u32;
                                self.emit(Opcode::DELETE_NAME, idx);
                            } else {
                                let idx = self.add_varname(name) as u32;
                                self.emit(Opcode::DELETE_FAST, idx);
                            }
                        }
                        _ => return Err("Can only delete simple names".to_string()),
                    }
                }
            }
            Stmt::Raise { exc, cause } => {
                if let Some(cause) = cause {
                    if let Some(exc) = exc {
                        self.compile_expr(exc)?;
                        self.compile_expr(cause)?;
                    } else {
                        return Err("Cannot raise with cause but no exception".to_string());
                    }
                    self.emit(Opcode::RAISE_VARARGS, 2);
                } else if let Some(exc) = exc {
                    self.compile_expr(exc)?;
                    self.emit(Opcode::RAISE_VARARGS, 1);
                } else {
                    self.emit(Opcode::RAISE_VARARGS, 0);
                }
            }
            Stmt::Try { body, handlers, orelse, finalbody } => {
                if !finalbody.is_empty() && handlers.is_empty() && orelse.is_empty() {
                    // Simple try/finally
                    let finally_label = self.new_label();
                    let end_label = self.new_label();
                    self.emit_jump(Opcode::SETUP_FINALLY, finally_label);
                    self.compile_stmts(body)?;
                    self.emit(Opcode::POP_BLOCK, 0);
                    self.compile_stmts(finalbody)?;
                    self.emit_jump(Opcode::JUMP, end_label);
                    self.fix_label(finally_label);
                    self.emit(Opcode::PUSH_EXC_INFO, 0);
                    self.compile_stmts(finalbody)?;
                    self.emit(Opcode::POP_EXCEPT, 0);
                    self.emit(Opcode::RERAISE, 0);
                    self.fix_label(end_label);
                } else if !finalbody.is_empty() {
                    // try/except/finally: wrap except handlers in a finally
                    let finally_label = self.new_label();
                    let end_label = self.new_label();
                    self.emit_jump(Opcode::SETUP_FINALLY, finally_label);
                    let cleanup = self.new_label();
                    self.emit_jump(Opcode::SETUP_FINALLY, cleanup);
                    let body_end = self.new_label();
                    let handler_done = self.new_label();
                    self.compile_stmts(body)?;
                    self.emit(Opcode::POP_BLOCK, 0);
                    self.emit_jump(Opcode::JUMP, body_end);
                    self.fix_label(cleanup);
                    self.emit(Opcode::PUSH_EXC_INFO, 0);
                    for handler in handlers {
                        if let Some(typ) = &handler.typ {
                            self.emit(Opcode::DUP_TOP, 0);
                            self.compile_expr(typ)?;
                            self.emit(Opcode::CHECK_EXC_MATCH, 0);
                            let next_handler = self.new_label();
                            self.emit_jump(Opcode::POP_JUMP_IF_FALSE, next_handler);
                            if let Some(name) = &handler.name {
                                let idx = self.add_varname(name) as u32;
                                self.emit(Opcode::STORE_FAST, idx);
                            }
                            self.compile_stmts(&handler.body)?;
                            self.emit_jump(Opcode::JUMP, handler_done);
                            self.fix_label(next_handler);
                        } else {
                            if let Some(name) = &handler.name {
                                let idx = self.add_varname(name) as u32;
                                self.emit(Opcode::STORE_FAST, idx);
                            }
                            self.compile_stmts(&handler.body)?;
                            self.emit_jump(Opcode::JUMP, handler_done);
                        }
                    }
                    self.emit(Opcode::RERAISE, 0);
                    self.fix_label(handler_done);
                    self.emit(Opcode::POP_EXCEPT, 0);
                    self.fix_label(body_end);
                    if !orelse.is_empty() {
                        self.compile_stmts(orelse)?;
                    }
                    self.emit(Opcode::POP_BLOCK, 0);
                    self.compile_stmts(finalbody)?;
                    self.emit_jump(Opcode::JUMP, end_label);
                    self.fix_label(finally_label);
                    self.emit(Opcode::PUSH_EXC_INFO, 0);
                    self.compile_stmts(finalbody)?;
                    self.emit(Opcode::POP_EXCEPT, 0);
                    self.emit(Opcode::RERAISE, 0);
                    self.fix_label(end_label);
                } else if !handlers.is_empty() {
                    let cleanup = self.new_label();
                    self.emit_jump(Opcode::SETUP_FINALLY, cleanup);
                    let body_end = self.new_label();
                    let handler_done = self.new_label();
                    self.compile_stmts(body)?;
                    self.emit(Opcode::POP_BLOCK, 0);
                    self.emit_jump(Opcode::JUMP, body_end);
                    self.fix_label(cleanup);
                    self.emit(Opcode::PUSH_EXC_INFO, 0);
                    for handler in handlers {
                        if let Some(typ) = &handler.typ {
                            self.emit(Opcode::DUP_TOP, 0);
                            self.compile_expr(typ)?;
                            self.emit(Opcode::CHECK_EXC_MATCH, 0);
                            let next_handler = self.new_label();
                            self.emit_jump(Opcode::POP_JUMP_IF_FALSE, next_handler);
                            if let Some(name) = &handler.name {
                                let idx = self.add_varname(name) as u32;
                                self.emit(Opcode::STORE_FAST, idx);
                            }
                            self.compile_stmts(&handler.body)?;
                            self.emit_jump(Opcode::JUMP, handler_done);
                            self.fix_label(next_handler);
                        } else {
                            if let Some(name) = &handler.name {
                                let idx = self.add_varname(name) as u32;
                                self.emit(Opcode::STORE_FAST, idx);
                            }
                            self.compile_stmts(&handler.body)?;
                            self.emit_jump(Opcode::JUMP, handler_done);
                        }
                    }
                    self.emit(Opcode::RERAISE, 0);
                    self.fix_label(handler_done);
                    self.emit(Opcode::POP_EXCEPT, 0);
                    self.fix_label(body_end);
                    if !orelse.is_empty() {
                        self.compile_stmts(orelse)?;
                    }
                } else {
                    self.compile_stmts(body)?;
                }
            }
            Stmt::Assert { test, msg } => {
                let assertion_error_idx = self.get_const_index(ConstValue::String("AssertionError".to_string())) as u32;
                self.emit(Opcode::LOAD_CONST, assertion_error_idx);
                self.compile_expr(test)?;
                let ok_label = self.new_label();
                self.emit_jump(Opcode::POP_JUMP_IF_TRUE, ok_label);
                if let Some(msg) = msg {
                    self.compile_expr(msg)?;
                } else {
                    let const_none = self.get_const_index(ConstValue::None) as u32;
                    self.emit(Opcode::LOAD_CONST, const_none);
                }
                self.compile_expr(test)?;
                let mut args = 1;
                if msg.is_some() {
                    args = 2;
                }
                self.emit(Opcode::CALL, args);
                self.emit(Opcode::RAISE_VARARGS, 1);
                self.fix_label(ok_label);
                self.emit(Opcode::POP_TOP, 0);
            }
            Stmt::With { items, body } => {
                for (i, item) in items.iter().enumerate() {
                    self.compile_expr(&item.context_expr)?;
                    self.emit(Opcode::SETUP_WITH, 0);
                    if let Some(var) = &item.optional_vars {
                        self.compile_assign_target(var)?;
                    } else {
                        self.emit(Opcode::POP_TOP, 0);
                    }
                }
                if items.len() == 1 {
                    // Use try/finally to ensure __exit__ is called on exception
                    let finally_label = self.new_label();
                    let end_label = self.new_label();
                    self.emit_jump(Opcode::SETUP_FINALLY, finally_label);
                    self.compile_stmts(body)?;
                    self.emit(Opcode::POP_BLOCK, 0);
                    self.compile_expr(&items[0].context_expr)?;
                    let exit_name_idx = self.get_name_index("__exit__") as u32;
                    self.emit(Opcode::LOAD_ATTR, exit_name_idx);
                    let const_none = self.get_const_index(ConstValue::None) as u32;
                    for _ in 0..3 {
                        self.emit(Opcode::LOAD_CONST, const_none);
                    }
                    self.emit(Opcode::CALL, 3);
                    self.emit(Opcode::POP_TOP, 0);
                    self.emit_jump(Opcode::JUMP, end_label);
                    self.fix_label(finally_label);
                    self.emit(Opcode::PUSH_EXC_INFO, 0);
                    self.compile_expr(&items[0].context_expr)?;
                    let exit_idx = self.get_name_index("__exit__") as u32;
                    self.emit(Opcode::LOAD_ATTR, exit_idx);
                    let const_none = self.get_const_index(ConstValue::None) as u32;
                    for _ in 0..3 {
                        self.emit(Opcode::LOAD_CONST, const_none);
                    }
                    self.emit(Opcode::CALL, 3);
                    self.emit(Opcode::POP_TOP, 0);
                    self.emit(Opcode::POP_EXCEPT, 0);
                    self.emit(Opcode::RERAISE, 0);
                    self.fix_label(end_label);
                } else {
                    self.compile_stmts(body)?;
                }
            }
            Stmt::AnnAssign { target, annotation: _, value } => {
                if let Some(val) = value {
                    self.compile_expr(val)?;
                    self.compile_assign_target(target)?;
                }
            }
            Stmt::Match { subject, cases } => {
                self.compile_expr(subject)?;
                let end_label = self.new_label();
                for case in cases {
                    let next_case = self.new_label();
                    // We need to match patterns. For simplicity, compile as if-elif chain.
                    // Match subject, compare with pattern value (simple match value only)
                    match &case.pattern {
                        Pattern::MatchValue(val) => {
                            self.emit(Opcode::DUP_TOP, 0);
                            self.compile_expr(val)?;
                            self.emit(Opcode::COMPARE_OP, 2); // ==
                            if case.guard.is_some() {
                                let guard_false = self.new_label();
                                self.emit_jump(Opcode::POP_JUMP_IF_FALSE, guard_false);
                                let guard = case.guard.as_ref().unwrap();
                                self.compile_expr(guard)?;
                                self.emit_jump(Opcode::POP_JUMP_IF_FALSE, next_case);
                                self.fix_label(guard_false);
                            } else {
                                self.emit_jump(Opcode::POP_JUMP_IF_FALSE, next_case);
                            }
                        }
                        Pattern::MatchAs { name: Some(n), .. } => {
                            let idx = self.add_varname(n) as u32;
                            self.emit(Opcode::DUP_TOP, 0);
                            self.emit(Opcode::STORE_FAST, idx);
                            if let Some(guard) = &case.guard {
                                self.compile_expr(guard)?;
                                self.emit_jump(Opcode::POP_JUMP_IF_FALSE, next_case);
                            }
                        }
                        Pattern::MatchAs { name: None, .. } => {
                            // Wildcard: always matches
                            if let Some(guard) = &case.guard {
                                self.compile_expr(guard)?;
                                self.emit_jump(Opcode::POP_JUMP_IF_FALSE, next_case);
                            }
                        }
                        Pattern::MatchSingleton(s) => {
                            self.emit(Opcode::DUP_TOP, 0);
                            let const_idx = self.get_const_index(match s.as_str() {
                                "None" => ConstValue::None,
                                "True" => ConstValue::Bool(true),
                                "False" => ConstValue::Bool(false),
                                _ => ConstValue::String(s.clone()),
                            }) as u32;
                            self.emit(Opcode::LOAD_CONST, const_idx);
                            self.emit(Opcode::COMPARE_OP, 8); // IS
                            if let Some(guard) = &case.guard {
                                let guard_false = self.new_label();
                                self.emit_jump(Opcode::POP_JUMP_IF_FALSE, guard_false);
                                self.compile_expr(guard)?;
                                self.emit_jump(Opcode::POP_JUMP_IF_FALSE, next_case);
                                self.fix_label(guard_false);
                            } else {
                                self.emit_jump(Opcode::POP_JUMP_IF_FALSE, next_case);
                            }
                        }
                        _ => return Err("Match pattern not supported yet".to_string()),
                    }
                    self.emit(Opcode::POP_TOP, 0); // pop subject
                    self.compile_stmts(&case.body)?;
                    self.emit_jump(Opcode::JUMP, end_label);
                    self.fix_label(next_case);
                    // Re-push subject for next case
                    self.emit(Opcode::COPY, 0);
                }
                self.emit(Opcode::POP_TOP, 0); // pop subject at end
                self.fix_label(end_label);
            }
        }
        Ok(())
    }

    fn compile_assign_target(&mut self, target: &Expr) -> Result<(), String> {
        match target {
            Expr::Name(name) => {
                if self.scope == ScopeType::Module || self.scope == ScopeType::ClassBody || self.global_names.contains(name) {
                    let idx = self.get_name_index(name) as u32;
                    self.emit(Opcode::STORE_NAME, idx);
                } else {
                    let idx = self.add_varname(name) as u32;
                    self.emit(Opcode::STORE_FAST, idx);
                }
            }
            Expr::Attribute { value, attr } => {
                self.compile_expr(value)?;
                let idx = self.get_name_index(attr) as u32;
                self.emit(Opcode::STORE_ATTR, idx);
            }
            Expr::Subscript { value, slice } => {
                self.compile_expr(value)?;
                self.compile_expr(slice)?;
                self.emit(Opcode::STORE_SUBSCR, 0);
            }
            Expr::List(elts) | Expr::Tuple(elts) => {
                let count = elts.len();
                self.emit(Opcode::UNPACK_SEQUENCE, count as u32);
                for elt in elts {
                    self.compile_assign_target(elt)?;
                }
            }
            _ => return Err(format!("Cannot assign to {:?}", target)),
        }
        Ok(())
    }

    fn compile_function(&mut self, name: String, args: &[Arg], body: &[Stmt]) -> Result<(), String> {
        // Extract docstring from first statement if present
        let docstring = body.first().and_then(|s| {
            if let Stmt::Expr(expr) = s {
                if let Expr::Constant(Constant::String(doc)) = expr.as_ref() {
                    Some(doc.clone())
                } else { None }
            } else { None }
        });
        let body = if docstring.is_some() { &body[1..] } else { body };

        self.enter_scope(ScopeType::Function);

        let old_code = std::mem::replace(&mut self.code, CodeObject::new(name.clone()));
        let old_labels = std::mem::replace(&mut self.labels, Vec::new());
        let old_label_stack = std::mem::replace(&mut self.label_stack, Vec::new());
        let old_loop_stack = std::mem::replace(&mut self.loop_stack, Vec::new());

        for arg in args {
            self.add_varname(&arg.arg);
        }
        self.code.arg_count = args.len();

        self.compile_stmts(body)?;

        // Implicit return None
        let has_return = body.iter().any(|s| matches!(s, Stmt::Return(_)));
        if !has_return {
            let const_none = self.get_const_index(ConstValue::None) as u32;
            self.emit(Opcode::LOAD_CONST, const_none);
            self.emit(Opcode::RETURN_VALUE, 0);
        }

        self.code.nlocals = self.code.varnames.len();
        self.code.name = name.clone();
        self.code.first_lineno = 1;

        let func_code = std::mem::replace(&mut self.code, old_code);
        self.labels = old_labels;
        self.label_stack = old_label_stack;
        self.loop_stack = old_loop_stack;

        let code_const_idx = self.get_const_index(ConstValue::Code(Box::new(func_code))) as u32;
        self.emit(Opcode::LOAD_CONST, code_const_idx);
        self.emit(Opcode::MAKE_FUNCTION, 0);

        // Set __doc__ if there was a docstring
        if let Some(doc) = docstring {
            self.emit(Opcode::DUP_TOP, 0);
            let doc_idx = self.get_const_index(ConstValue::String(doc)) as u32;
            self.emit(Opcode::LOAD_CONST, doc_idx);
            let doc_attr_idx = self.get_name_index("__doc__") as u32;
            self.emit(Opcode::STORE_ATTR, doc_attr_idx);
        }

        // Handle closure cells if needed
        let cell_count = self.code.cellvars.len() as u32;
        if cell_count > 0 {
            for i in 0..cell_count {
                self.emit(Opcode::LOAD_CLOSURE, i);
            }
            self.emit(Opcode::BUILD_TUPLE, cell_count);
            let const_none = self.get_const_index(ConstValue::None) as u32;
            self.emit(Opcode::LOAD_CONST, const_none);
            self.emit(Opcode::LOAD_CONST, const_none);
            self.emit(Opcode::CALL, 3);
        }

        self.leave_scope();
        Ok(())
    }

    fn compile_class_body(&mut self, name: String, body: &[Stmt]) -> Result<(), String> {
        // Skip docstring if first statement is a string literal
        let body = if let Some(Stmt::Expr(expr)) = body.first() {
            if matches!(expr.as_ref(), Expr::Constant(Constant::String(_))) {
                &body[1..]
            } else { body }
        } else { body };

        self.enter_scope(ScopeType::ClassBody);

        let old_code = std::mem::replace(&mut self.code, CodeObject::new(name.clone()));
        let old_labels = std::mem::replace(&mut self.labels, Vec::new());
        let old_label_stack = std::mem::replace(&mut self.label_stack, Vec::new());
        let old_loop_stack = std::mem::replace(&mut self.loop_stack, Vec::new());

        self.code.arg_count = 0;

        self.compile_stmts(body)?;

        let const_none = self.get_const_index(ConstValue::None) as u32;
        self.emit(Opcode::LOAD_CONST, const_none);
        self.emit(Opcode::RETURN_VALUE, 0);

        self.code.nlocals = self.code.varnames.len();
        self.code.name = name.clone();
        self.code.first_lineno = 1;

        let func_code = std::mem::replace(&mut self.code, old_code);
        self.labels = old_labels;
        self.label_stack = old_label_stack;
        self.loop_stack = old_loop_stack;

        let code_const_idx = self.get_const_index(ConstValue::Code(Box::new(func_code))) as u32;
        self.emit(Opcode::LOAD_CONST, code_const_idx);
        self.emit(Opcode::MAKE_FUNCTION, 0);

        self.leave_scope();
        Ok(())
    }

    // ---- Expression compilation ----

    fn compile_expr(&mut self, expr: &Expr) -> Result<(), String> {
        match expr {
            Expr::Constant(c) => {
                let const_value = match c {
                    Constant::None => ConstValue::None,
                    Constant::Bool(b) => ConstValue::Bool(*b),
                    Constant::Int(s) => ConstValue::Int(s.clone()),
                    Constant::Float(s) => ConstValue::Float(s.clone()),
                    Constant::String(s) => ConstValue::String(s.clone()),
                    Constant::Ellipsis => ConstValue::String("...".to_string()),
                    Constant::Bytes(_) => ConstValue::None,
                    Constant::Complex { real: _, imag: _ } => ConstValue::None,
                };
                let idx = self.get_const_index(const_value) as u32;
                self.emit(Opcode::LOAD_CONST, idx);
            }
            Expr::Name(name) => {
                if self.scope == ScopeType::Module || self.scope == ScopeType::ClassBody || self.global_names.contains(name) {
                    let name_idx = self.get_name_index(name) as u32;
                    self.emit(Opcode::LOAD_NAME, name_idx);
                } else if self.scope == ScopeType::Function && self.get_var_index(name).is_some() {
                    let idx = self.get_var_index(name).unwrap() as u32;
                    self.emit(Opcode::LOAD_FAST, idx);
                } else if self.scope == ScopeType::Function && self.code.cellvars.contains(name) {
                    let idx = self.code.cellvars.iter().position(|n| n == name).unwrap() as u32;
                    self.emit(Opcode::LOAD_DEREF, idx);
                } else if self.scope == ScopeType::Function {
                    let name_idx = self.get_name_index(name) as u32;
                    self.emit(Opcode::LOAD_GLOBAL, name_idx);
                } else {
                    let name_idx = self.get_name_index(name) as u32;
                    self.emit(Opcode::LOAD_NAME, name_idx);
                }
            }
            Expr::BinOp { left, op, right } => {
                self.compile_expr(left)?;
                self.compile_expr(right)?;
                let bin_op = match op {
                    Operator::Add => 0,
                    Operator::Sub => 1,
                    Operator::Mult => 2,
                    Operator::Div => 3,
                    Operator::FloorDiv => 4,
                    Operator::Mod => 5,
                    Operator::Pow => 6,
                    Operator::LShift => 7,
                    Operator::RShift => 8,
                    Operator::BitOr => 9,
                    Operator::BitXor => 10,
                    Operator::BitAnd => 11,
                    Operator::MatMult => 12,
                };
                self.emit(Opcode::BINARY_OP, bin_op);
            }
            Expr::UnaryOp { op, operand } => {
                self.compile_expr(operand)?;
                match op {
                    UnaryOp::Not => self.emit(Opcode::UNARY_NOT, 0),
                    UnaryOp::UAdd => self.emit(Opcode::UNARY_NEGATIVE, 0),
                    UnaryOp::USub => self.emit(Opcode::UNARY_NEGATIVE, 0),
                    UnaryOp::Invert => self.emit(Opcode::UNARY_INVERT, 0),
                }
            }
            Expr::BoolOp { op, values } => {
                let end_label = self.new_label();
                let (jump_op, short_circuit) = match op {
                    BoolOp::And => (Opcode::POP_JUMP_IF_FALSE, ConstValue::Bool(false)),
                    BoolOp::Or => (Opcode::POP_JUMP_IF_TRUE, ConstValue::Bool(true)),
                };
                let short_val_idx = self.get_const_index(short_circuit) as u32;
                for (i, val) in values.iter().enumerate() {
                    if i > 0 {
                        // Emit target: NOP label
                    }
                    self.compile_expr(val)?;
                    if i < values.len() - 1 {
                        self.emit_jump(jump_op, end_label);
                        self.emit(Opcode::POP_TOP, 0);
                    }
                }
                self.fix_label(end_label);
            }
            Expr::Compare { left, ops, comparators } => {
                self.compile_expr(left)?;
                for (i, (op, right)) in ops.iter().zip(comparators.iter()).enumerate() {
                    if i > 0 {
                        // For chained comparisons, need DUP and ROT
                        return Err("Chained comparisons not yet supported".to_string());
                    }
                    self.compile_expr(right)?;
                    let cmp_op = match op {
                        CmpOp::Eq => 2,
                        CmpOp::NotEq => 5,
                        CmpOp::Lt => 0,
                        CmpOp::LtE => 1,
                        CmpOp::Gt => 4,
                        CmpOp::GtE => 3,
                        CmpOp::Is => 8,
                        CmpOp::IsNot => 9,
                        CmpOp::In => 6,
                        CmpOp::NotIn => 7,
                    };
                    self.emit(Opcode::COMPARE_OP, cmp_op);
                }
            }
            Expr::Call { func, args, keywords } => {
                let total_args = args.len() + keywords.len();

                self.compile_expr(func)?;

                for arg in args {
                    self.compile_expr(arg)?;
                }
                for kw in keywords {
                    if let Some(name) = &kw.arg {
                        // Keyword argument
                        self.compile_expr(&kw.value)?;
                    } else {
                        // **kwargs
                        self.compile_expr(&kw.value)?;
                    }
                }
                if keywords.is_empty() {
                    self.emit(Opcode::CALL, total_args as u32);
                } else {
                    // For keywords, we need CALL_KW - for now do CALL with keyword args too
                    self.emit(Opcode::CALL, total_args as u32);
                }
            }
            Expr::IfExp { test, body, orelse } => {
                self.compile_expr(test)?;
                let else_label = self.new_label();
                let end_label = self.new_label();
                self.emit_jump(Opcode::POP_JUMP_IF_FALSE, else_label);
                self.compile_expr(body)?;
                self.emit_jump(Opcode::JUMP, end_label);
                self.fix_label(else_label);
                self.compile_expr(orelse)?;
                self.fix_label(end_label);
            }
            Expr::Lambda { args, body } => {
                self.compile_function("<lambda>".to_string(), args, &[Stmt::Return(Some(body.clone()))])?;
            }
            Expr::Attribute { value, attr } => {
                self.compile_expr(value)?;
                let idx = self.get_name_index(attr) as u32;
                self.emit(Opcode::LOAD_ATTR, idx);
            }
            Expr::Subscript { value, slice } => {
                self.compile_expr(value)?;
                self.compile_expr(slice)?;
                self.emit(Opcode::BINARY_OP, 13); // SUBSCR = 13
            }
            Expr::List(elts) => {
                for elt in elts {
                    self.compile_expr(elt)?;
                }
                self.emit(Opcode::BUILD_LIST, elts.len() as u32);
            }
            Expr::Tuple(elts) => {
                for elt in elts {
                    self.compile_expr(elt)?;
                }
                self.emit(Opcode::BUILD_TUPLE, elts.len() as u32);
            }
            Expr::Dict { keys, values } => {
                self.emit(Opcode::BUILD_MAP, keys.len() as u32);
                for (key, value) in keys.iter().zip(values.iter()) {
                    self.emit(Opcode::DUP_TOP, 0);
                    match key {
                        Some(k) => self.compile_expr(k)?,
                        None => return Err("Dict key expected".to_string()),
                    }
                    self.compile_expr(value)?;
                    self.emit(Opcode::MAP_ADD, 1);
                }
            }
            Expr::Set(elts) => {
                for elt in elts {
                    self.compile_expr(elt)?;
                }
                self.emit(Opcode::BUILD_SET, elts.len() as u32);
            }
            Expr::ListComp { elt, generators } => {
                self.compile_comprehension(elt, generators, false)?;
            }
            Expr::SetComp { elt, generators } => {
                self.compile_comprehension(elt, generators, true)?;
            }
            Expr::GeneratorExp { elt, generators } => {
                self.compile_comprehension(elt, generators, false)?;
            }
            Expr::Slice { lower, upper, step } => {
                let const_none = self.get_const_index(ConstValue::None) as u32;
                if let Some(l) = lower { self.compile_expr(l)?; }
                else { self.emit(Opcode::LOAD_CONST, const_none); }
                if let Some(u) = upper { self.compile_expr(u)?; }
                else { self.emit(Opcode::LOAD_CONST, const_none); }
                if let Some(s) = step { self.compile_expr(s)?; }
                else { self.emit(Opcode::LOAD_CONST, const_none); }
                if step.is_some() {
                    self.emit(Opcode::BUILD_SLICE, 3);
                } else {
                    self.emit(Opcode::BUILD_SLICE, 2);
                }
            }
            Expr::Starred(expr) => {
                self.compile_expr(expr)?;
            }
            Expr::Yield(Some(expr)) => {
                self.compile_expr(expr)?;
                self.emit(Opcode::YIELD_VALUE, 0);
            }
            Expr::Yield(None) => {
                let const_none = self.get_const_index(ConstValue::None) as u32;
                self.emit(Opcode::LOAD_CONST, const_none);
                self.emit(Opcode::YIELD_VALUE, 0);
            }
            Expr::FString(parts) => {
                let mut parts_to_build = Vec::new();
                for part in parts {
                    match part {
                        FStringPart::String(s) => {
                            parts_to_build.push(Expr::Constant(Constant::String(s.clone())));
                        }
                        FStringPart::Expr(expr) => {
                            self.compile_expr(expr)?;
                            parts_to_build.push(Expr::Constant(Constant::String("".to_string())));
                        }
                    }
                }
                // Build concatenated string
                if parts_to_build.len() == 1 {
                    self.compile_expr(&parts_to_build[0])?;
                } else {
                    for part in &parts_to_build {
                        self.compile_expr(part)?;
                    }
                    self.emit(Opcode::BUILD_STRING, parts_to_build.len() as u32);
                }
            }
            Expr::JoinedStr(parts) => {
                for part in parts {
                    self.compile_expr(part)?;
                }
                if parts.len() == 1 {
                    // Already on stack
                } else {
                    self.emit(Opcode::BUILD_STRING, parts.len() as u32);
                }
            }
            Expr::NamedExpr { target, value } => {
                self.compile_expr(value)?;
                self.emit(Opcode::COPY, 1);
                self.compile_assign_target(target)?;
            }
            Expr::DictComp { .. } => {
                return Err("Dict comprehensions not yet supported".to_string());
            }
            Expr::Await(_) | Expr::YieldFrom(_) => {
                return Err("Await/yield from not supported yet".to_string());
            }
        }
        Ok(())
    }

    fn compile_comprehension(&mut self, elt: &Expr, generators: &[Comprehension], is_set: bool) -> Result<(), String> {
        if generators.len() != 1 {
            return Err("Only single-generator comprehensions supported".to_string());
        }
        let gen = &generators[0];

        // Build result container
        if is_set {
            self.emit(Opcode::BUILD_SET, 0);
        } else {
            self.emit(Opcode::BUILD_LIST, 0);
        }

        let start_label = self.new_label();
        let end_label = self.new_label();
        let comp_start_label = self.new_label();

        self.compile_expr(&gen.iter)?;
        self.emit(Opcode::GET_ITER, 0);
        self.loop_stack.push(LoopInfo { start_label: comp_start_label, end_label });
        self.mark_label(comp_start_label);
        self.emit_jump(Opcode::FOR_ITER, end_label);
        self.compile_assign_target(&gen.target)?;

        // Ifs — if any condition is false, skip the elt (jump to next_iteration)
        let next_iteration = self.new_label();
        for if_expr in &gen.ifs {
            self.compile_expr(if_expr)?;
            self.emit_jump(Opcode::POP_JUMP_IF_FALSE, next_iteration);
        }

        self.compile_expr(elt)?;
        if is_set {
            self.emit(Opcode::SET_ADD, 1);
        } else {
            self.emit(Opcode::LIST_APPEND, 1);
        }

        self.fix_label(next_iteration);
        self.emit_backward_jump(comp_start_label);

        self.fix_label(end_label);
        self.emit(Opcode::POP_ITER, 0);
        self.loop_stack.pop();

        Ok(())
    }
}
