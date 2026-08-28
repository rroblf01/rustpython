use crate::ast::*;
use crate::bytecode::*;
use std::collections::HashSet;

mod closure;
mod comprehension;
mod expr;
mod function;
mod scope;
mod stmt;
mod utils;
pub use scope::{LoopInfo, PendingCleanup, ScopeInfo, ScopeType};
pub use utils::{contains_yield_in_expr, contains_yield_in_stmts, delete_error_for, stmt_has_top_level_await};

pub struct Compiler {
    code: CodeObject,
    labels: Vec<Vec<usize>>,
    label_positions: Vec<usize>,
    label_stack: Vec<Vec<(usize, u32)>>,
    loop_stack: Vec<LoopInfo>,
    // Active `with`-blocks and `try`/`finally` blocks currently being
    // compiled (within the current function scope only — reset per function
    // like loop_stack). A `return` compiled while this is non-empty must
    // inline, innermost first, either the `__exit__`/`__aexit__` call (With)
    // or a fresh copy of the `finally` body (Finally) for each entry before
    // the actual RETURN_VALUE: CPython itself does this at compile time
    // rather than having the VM unwind pending with/finally blocks on early
    // return, and this VM's RETURN_VALUE never did the latter either — so
    // without this, `with cm(): return x` silently skipped `__exit__`
    // entirely, and (a separate, more fundamental gap found later) so did
    // plain `try: return x finally: ...` skip its own `finally` block
    // completely, for every single `return`-inside-`try` in the entire
    // codebase's history — confirmed via the simplest possible repro (`def
    // f():\n try: return "ret"\n finally: print("ran")` printing nothing).
    pending_cleanup: Vec<PendingCleanup>,
    scope: ScopeType,
    global_names: HashSet<String>,
    nonlocal_names: HashSet<String>,
    scope_stack: Vec<ScopeInfo>,
    // Parallel to scope_stack: the varnames of the code object that was
    // active immediately before each nested scope was entered. Lets us walk
    // past intervening class-body scopes to find the nearest real enclosing
    // function scope for closure-variable resolution.
    varnames_stack: Vec<Vec<String>>,
    // Parallel to scope_stack's ClassBody entries: the name of the class
    // currently being defined. Used to resolve bare `super()` (PEP 3135) —
    // see the Expr::Call bare-super compilation for why this must be the
    // class the method is textually defined in, not type(self).
    class_name_stack: Vec<String>,
    current_line: usize,
    // Whether `__annotations__` has already been created (via BUILD_MAP +
    // STORE_NAME) in the CURRENT module/class scope — reset per scope like
    // `current_line`. Only consulted for `Stmt::AnnAssign` at Module/
    // ClassBody scope (see there); Function scope never populates a real
    // `__annotations__`, matching real Python (local variable annotations
    // are evaluated for side effects only, never stored anywhere).
    annotations_initialized: bool,
    comprehension_depth: usize,
}

impl Compiler {
    pub fn new() -> Self {
        Compiler {
            code: CodeObject::new("<module>"),
            labels: Vec::new(),
            label_positions: Vec::new(),
            label_stack: Vec::new(),
            loop_stack: Vec::new(),
            pending_cleanup: Vec::new(),
            scope: ScopeType::Module,
            global_names: HashSet::new(),
            nonlocal_names: HashSet::new(),
            scope_stack: Vec::new(),
            varnames_stack: Vec::new(),
            class_name_stack: Vec::new(),
            current_line: 1,
            annotations_initialized: false,
            comprehension_depth: 0,
        }
    }

    pub fn compile(&mut self, program: &Program, filename: &str) -> Result<CodeObject, String> {
        self.code.filename = crate::interner::intern(filename);
        // Ensure constant 0 is always None for module return
        if self.code.consts.is_empty() || !matches!(&self.code.consts[0], ConstValue::None) {
            self.code.consts.insert(0, ConstValue::None);
        }
        match program {
            Program::Module(stmts) => {
                // Top-level `await`/`async for`/`async with` (enabled by the
                // PyCF_ALLOW_TOP_LEVEL_AWAIT compile flag) makes the module a
                // coroutine — CPython sets CO_COROUTINE (0x80; this VM also
                // uses its own 0x100 bit) and RETURN_GENERATOR so the code
                // can be awaited. test_builtin::test_compile_top_level_await
                // checks co_flags & CO_COROUTINE on such modules.
                let has_top_await = stmts.iter().any(stmt_has_top_level_await);
                if has_top_await {
                    self.code.flags |= 0x180;
                    self.emit(Opcode::RETURN_GENERATOR, 0);
                }
                // A bare string-literal expression as the module's FIRST
                // statement is its docstring, matching real CPython's
                // module `__doc__` semantics — this was never handled at
                // all (only function/class docstrings were), so EVERY
                // module's `__doc__` was unconditionally `None` regardless
                // of what the module's own source actually said. Confirmed
                // via the simplest repro: `"""doc"""` as a module's first
                // line still left `module.__doc__` as `None`. Compiled the
                // same way real CPython does — a direct `STORE_NAME
                // __doc__`, not a discarded expression statement.
                let mut start = 0;
                if let Some(Stmt::Expr(expr)) = stmts.first().map(Self::unwrap_located) {
                    if let Expr::Constant(Constant::String(doc)) = expr.as_ref() {
                        let doc_idx = self.get_const_index(ConstValue::String(doc.clone())) as u32;
                        self.emit(Opcode::LOAD_CONST, doc_idx);
                        let doc_name_idx = self.get_name_index("__doc__") as u32;
                        self.emit(Opcode::STORE_NAME, doc_name_idx);
                        start = 1;
                    }
                }
                self.compile_stmts(&stmts[start..])?;
            }
            Program::Expression(expr) => {
                self.compile_expr(expr)?;
                self.emit(Opcode::RETURN_VALUE, 0);
            }
        }
        if self.scope == ScopeType::Module {
            self.code.nlocals = self.code.varnames.len();
            self.emit(Opcode::LOAD_CONST, 0);
            self.emit(Opcode::RETURN_VALUE, 0);
        }
        // Remove NOP instructions (dead code elimination)
        self.code.instructions.retain(|i| i.op != Opcode::NOP);

        // Peephole fusion of hot straight-line sequences into
        // superinstructions (see superinstr.rs). Runs after dead-code
        // elimination so the NOPs it introduces are the only ones present.
        crate::superinstr::apply(&mut self.code);

        Ok(self.code.clone())
    }

    fn enter_scope(&mut self, scope: ScopeType) {
        let info = ScopeInfo {
            scope: self.scope.clone(),
            global_names: std::mem::take(&mut self.global_names),
            nonlocal_names: std::mem::take(&mut self.nonlocal_names),
        };
        self.scope_stack.push(info);
        self.scope = scope;
    }

    fn leave_scope(&mut self) {
        if let Some(info) = self.scope_stack.pop() {
            self.scope = info.scope;
            self.global_names = info.global_names;
            self.nonlocal_names = info.nonlocal_names;
        }
    }

    /// Names visible as closure candidates from the nearest REAL enclosing
    /// function scope, skipping over any class-body scopes in between (class
    /// bodies don't participate in Python's closure lookup chain, but a
    /// method nested inside one still needs to see past it to the function
    /// that actually encloses the class).
    /// Names a scope makes available to anything nested inside it: its plain
    /// locals/args, plus its cellvars and freevars. Cellvars are already
    /// mirrored into varnames, but freevars are not (they're only known to
    /// need a varnames slot if something relays them further, which is
    /// decided after this scope's body — and any further-nested scope — has
    /// already been compiled), so they must be listed explicitly here.
    fn enclosing_snapshot(code: &CodeObject) -> Vec<String> {
        let mut names: Vec<String> = code
            .varnames
            .iter()
            .map(|&id| crate::interner::lookup(id))
            .collect();
        names.extend(code.freevars.iter().cloned());
        names
    }

    fn compute_enclosing_names(&self) -> HashSet<String> {
        let mut idx = self.scope_stack.len();
        while idx > 0 {
            idx -= 1;
            if self.scope_stack[idx].scope != ScopeType::ClassBody {
                return self.varnames_stack[idx].iter().cloned().collect();
            }
        }
        HashSet::new()
    }

    fn get_var_index(&mut self, name: &str) -> Option<usize> {
        let interned = crate::interner::intern(name);
        self.code.varnames.iter().position(|&n| n == interned)
    }

    fn add_varname(&mut self, name: &str) -> usize {
        if let Some(idx) = self.get_var_index(name) {
            return idx;
        }
        self.code.varnames.push(crate::interner::intern(name));
        self.code.varnames.len() - 1
    }

    fn get_name_index(&mut self, name: &str) -> usize {
        let interned = crate::interner::intern(name);
        if let Some(idx) = self.code.names.iter().position(|&n| n == interned) {
            return idx;
        }
        self.code.names.push(crate::interner::intern(name));
        self.code.names.len() - 1
    }

    fn emit_load_name(&mut self, name: &str) {
        if self.scope == ScopeType::Module
            || self.scope == ScopeType::ClassBody
            || self.global_names.contains(name)
        {
            let name_idx = self.get_name_index(name) as u32;
            self.emit(Opcode::LOAD_NAME, name_idx);
        } else if self.scope == ScopeType::Function && self.code.freevars.contains(&name.to_string()) {
            let fv_idx = self.code.freevars.iter().position(|n| n == name).unwrap();
            let idx = self.code.cellvars.len() + fv_idx;
            self.emit(Opcode::LOAD_DEREF, idx as u32);
        } else if self.scope == ScopeType::Function && self.code.cellvars.contains(&name.to_string()) {
            let idx = self.code.cellvars.iter().position(|n| n == name).unwrap() as u32;
            self.emit(Opcode::LOAD_DEREF, idx);
        } else if self.scope == ScopeType::Function {
            if let Some(idx) = self.get_var_index(name) {
                self.emit(Opcode::LOAD_FAST, idx as u32);
            } else {
                let name_idx = self.get_name_index(name) as u32;
                self.emit(Opcode::LOAD_GLOBAL, name_idx);
            }
        } else {
            let name_idx = self.get_name_index(name) as u32;
            self.emit(Opcode::LOAD_NAME, name_idx);
        }
    }

    fn emit_store_name(&mut self, name: &str) {
        if self.scope == ScopeType::Module
            || self.scope == ScopeType::ClassBody
            || self.global_names.contains(name)
        {
            let idx = self.get_name_index(name) as u32;
            self.emit(Opcode::STORE_NAME, idx);
        } else if self.scope == ScopeType::Function && self.code.cellvars.contains(&name.to_string()) {
            let idx = self.code.cellvars.iter().position(|n| n == name).unwrap() as u32;
            self.emit(Opcode::STORE_DEREF, idx);
        } else if self.scope == ScopeType::Function && self.code.freevars.contains(&name.to_string()) {
            let fv_idx = self.code.freevars.iter().position(|n| n == name).unwrap();
            let idx = (self.code.cellvars.len() + fv_idx) as u32;
            self.emit(Opcode::STORE_DEREF, idx);
        } else if self.scope == ScopeType::Function {
            let idx = self.add_varname(name) as u32;
            self.emit(Opcode::STORE_FAST, idx);
        } else {
            let idx = self.get_name_index(name) as u32;
            self.emit(Opcode::STORE_NAME, idx);
        }
    }

    fn get_const_index(&mut self, c: ConstValue) -> usize {
        if let Some(idx) = self.code.consts.iter().position(|x| match (x, &c) {
            (ConstValue::None, ConstValue::None) => true,
            (ConstValue::Bool(a), ConstValue::Bool(b)) => a == b,
            (ConstValue::Int(a), ConstValue::Int(b)) => a == b,
            (ConstValue::Float(a), ConstValue::Float(b)) => a == b,
            (ConstValue::String(a), ConstValue::String(b)) => a == b,
            (ConstValue::Bytes(a), ConstValue::Bytes(b)) => a == b,
            _ => false,
        }) {
            return idx;
        }
        self.code.consts.push(c);
        self.code.consts.len() - 1
    }

    fn emit(&mut self, op: Opcode, arg: u32) {
        let instr = Instr::with_arg(op, arg);
        self.code.line_numbers.push(self.current_line as u32);
        self.code.instructions.push(instr);
    }

    fn set_line(&mut self, line: usize) {
        self.current_line = line;
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

    fn emit_label(&mut self, label: usize) {
        self.label_positions[label] = self.code.instructions.len();
    }

    fn emit_backward_jump(&mut self, target_label: usize) {
        let target = self.label_positions[target_label];
        let jump_pos = self.code.instructions.len();
        let offset = (jump_pos as u32).wrapping_sub(target as u32);
        self.emit(Opcode::JUMP_BACKWARD, offset);
    }

    // ---- Statement compilation ----

    /// Strips a `Stmt::Located` wrapper (added by the parser at each
    /// statement pushed into a block) down to the real statement. Statements
    /// synthesized by the compiler itself (e.g. multi-item `with` desugaring)
    /// are never wrapped and pass through unchanged.
    pub(crate) fn unwrap_located(stmt: &Stmt) -> &Stmt {
        match stmt {
            Stmt::Located(_, inner) => Self::unwrap_located(inner),
            _ => stmt,
        }
    }
}
