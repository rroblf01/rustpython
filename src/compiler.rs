use crate::ast::*;
use crate::bytecode::*;
use std::collections::HashSet;

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
}

#[derive(Clone)]
enum PendingCleanup {
    With(bool), // is_async
    Finally(Vec<Stmt>),
    // Marks "we're compiling an `except` handler's body, whose entry point
    // (`PUSH_EXC_INFO`) pushed the active exception onto the stack" — a
    // `return`/`break`/`continue` from inside that body must `POP_EXCEPT`
    // that pushed value before jumping out, exactly like the handler's own
    // normal fall-through path already does. Without this, `return` from
    // inside `except X: return val` (an extremely common pattern —
    // `import_fresh_module`'s own `except ImportError: return None`) left
    // the exception-info value permanently on the stack: harmless by
    // itself, but any ENCLOSING `with` block's return-cleanup inlining
    // (`PendingCleanup::With`, above) then swaps/dups/calls `__exit__` on
    // whatever's now on top of the stack — the stray exception object,
    // not the real context manager — surfacing as `AttributeError:
    // 'ImportError' object has no attribute '__exit__'` several statements
    // away from the actual bug.
    PopExcept,
}

struct LoopInfo {
    start_label: usize,
    end_label: usize,
    // `for`/`async for` loops keep their iterator object sitting on the
    // stack for the loop's whole duration (FOR_ITER peeks it each pass;
    // END_FOR pops it once on natural exhaustion, right before
    // `end_label`). A `break` jumps straight to `end_label`, skipping that
    // END_FOR — so without popping it here too, every `break` inside any
    // `for` loop permanently leaked one stack slot into the enclosing
    // frame, corrupting everything after it (confirmed: a `break` in a
    // `for` loop nested inside another `for`/`while` loop silently
    // desynced the outer loop's own iteration, either skipping values or
    // looping forever). `while` loops push nothing extra, so this stays
    // `false` there and `break` needs no compensating pop.
    is_for: bool,
}

struct ScopeInfo {
    scope: ScopeType,
    global_names: HashSet<String>,
    nonlocal_names: HashSet<String>,
}

#[derive(Clone, PartialEq, Debug)]
enum ScopeType {
    Module,
    Function,
    ClassBody,
}

impl Compiler {
    pub fn new() -> Self {
        Compiler {
            code: CodeObject::new("<module>".to_string()),
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
        let mut names: Vec<String> = code.varnames.iter().map(|&id| crate::interner::lookup(id)).collect();
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
        let mut instr = Instr::with_arg(op, arg);
        instr.line_no = Some(self.current_line);
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

    // ---- Closure analysis ----


    fn collect_names_expr(expr: &Expr, names: &mut HashSet<String>) {
        match expr {
            Expr::Name(n) => {
                names.insert(n.clone());
            }
            Expr::Constant(_) => {}
            // An f-string's embedded `{expr}`/`{expr:{format_spec}}` parts
            // are real expressions that can reference any name in scope —
            // including a closure variable from an enclosing function
            // (real code: `f"{func_name}() takes at most {max_positional_args}
            // ..."` inside a nested function, `django.utils.deprecation`).
            // Treating the whole f-string as opaque (the previous `=> {}`
            // no-op) made such names invisible to this scan, so the
            // upfront cell/free-variable analysis never learned the
            // enclosing function needed to expose them as cells — the same
            // class of bug as the control-flow-recursion fix above, just
            // triggered by a different AST shape.
            Expr::FString(parts) => {
                for part in parts {
                    if let FStringPart::Expr { expr, format_spec, .. } = part {
                        Self::collect_names_expr(expr, names);
                        if let Some(fs) = format_spec {
                            Self::collect_names_expr(fs, names);
                        }
                    }
                }
            }
            Expr::JoinedStr(exprs) => {
                for e in exprs {
                    Self::collect_names_expr(e, names);
                }
            }
            Expr::BinOp { left, right, .. } => {
                Self::collect_names_expr(left, names);
                Self::collect_names_expr(right, names);
            }
            Expr::UnaryOp { operand, .. } => Self::collect_names_expr(operand, names),
            Expr::BoolOp { values, .. } => {
                for v in values {
                    Self::collect_names_expr(v, names);
                }
            }
            Expr::Compare {
                left, comparators, ..
            } => {
                Self::collect_names_expr(left, names);
                for c in comparators {
                    Self::collect_names_expr(c, names);
                }
            }
            Expr::Call {
                func,
                args,
                keywords,
            } => {
                Self::collect_names_expr(func, names);
                for a in args {
                    Self::collect_names_expr(a, names);
                }
                for kw in keywords {
                    Self::collect_names_expr(&kw.value, names);
                }
            }
            Expr::IfExp { test, body, orelse } => {
                Self::collect_names_expr(test, names);
                Self::collect_names_expr(body, names);
                Self::collect_names_expr(orelse, names);
            }
            Expr::Attribute { value, .. } => Self::collect_names_expr(value, names),
            Expr::Subscript { value, slice } => {
                Self::collect_names_expr(value, names);
                Self::collect_names_expr(slice, names);
            }
            Expr::Starred(expr) => Self::collect_names_expr(expr, names),
            Expr::List(elts) | Expr::Tuple(elts) | Expr::Set(elts) => {
                for e in elts {
                    Self::collect_names_expr(e, names);
                }
            }
            Expr::Dict { keys, values } => {
                for k in keys.iter().flatten() {
                    Self::collect_names_expr(k, names);
                }
                for v in values {
                    Self::collect_names_expr(v, names);
                }
            }
            Expr::Slice { lower, upper, step } => {
                for s in [lower, upper, step].iter().filter_map(|s| s.as_ref()) {
                    Self::collect_names_expr(s, names);
                }
            }
            Expr::Lambda { body, .. } => Self::collect_names_expr(body, names),
            Expr::Yield(Some(e)) | Expr::YieldFrom(e) | Expr::Await(e) => {
                Self::collect_names_expr(e, names)
            }
            Expr::Yield(None) => {}
            // A comprehension/genexpr's `for target in ...` binds `target`
            // within its own scope — it is not a reference to anything
            // from the enclosing function, and must not be reported as
            // one. The previous code fed `gen.target` straight into the
            // same `names` set as everything else, so e.g. `any(... for
            // name in xs)` made "name" look like a free reference the
            // enclosing function needed to supply — which, once something
            // elsewhere also legitimately needed "name" relayed as a
            // closure (a `for name in ...` genexpr inside a *different*
            // nested function), caused the enclosing function's cellvars
            // list to gain an extra, unsorted, incrementally-added entry
            // for "name" *after* other cellvar-relative LOAD_DEREF indices
            // had already been emitted assuming a smaller list — silently
            // shifting them to the wrong variable (confirmed via
            // `django.utils.deprecation.deprecate_posargs`, whose `any(...
            // for name in remappable_names)` and a *separate* nested
            // function's own `for name in ...` genexpr collided exactly
            // this way). Only the first generator's `iter` is genuinely
            // evaluated in the enclosing scope (real Python semantics — it
            // becomes the genexpr's own initial argument); `elt`, every
            // other `iter`, and all `ifs` run inside the comprehension's
            // scope, where every generator's `target` is already bound, so
            // names matching one of those don't propagate outward either.
            Expr::ListComp { elt, generators }
            | Expr::SetComp { elt, generators }
            | Expr::GeneratorExp { elt, generators } => {
                let mut bound = HashSet::new();
                for gen in generators {
                    Self::collect_names_expr(&gen.target, &mut bound);
                }
                if let Some(first) = generators.first() {
                    Self::collect_names_expr(&first.iter, names);
                }
                let mut inner = HashSet::new();
                Self::collect_names_expr(elt, &mut inner);
                for (i, gen) in generators.iter().enumerate() {
                    if i > 0 {
                        Self::collect_names_expr(&gen.iter, &mut inner);
                    }
                    for if_cond in &gen.ifs {
                        Self::collect_names_expr(if_cond, &mut inner);
                    }
                }
                for n in inner {
                    if !bound.contains(&n) {
                        names.insert(n);
                    }
                }
            }
            Expr::DictComp {
                key,
                value,
                generators,
            } => {
                let mut bound = HashSet::new();
                for gen in generators {
                    Self::collect_names_expr(&gen.target, &mut bound);
                }
                if let Some(first) = generators.first() {
                    Self::collect_names_expr(&first.iter, names);
                }
                let mut inner = HashSet::new();
                Self::collect_names_expr(key, &mut inner);
                Self::collect_names_expr(value, &mut inner);
                for (i, gen) in generators.iter().enumerate() {
                    if i > 0 {
                        Self::collect_names_expr(&gen.iter, &mut inner);
                    }
                    for if_cond in &gen.ifs {
                        Self::collect_names_expr(if_cond, &mut inner);
                    }
                }
                for n in inner {
                    if !bound.contains(&n) {
                        names.insert(n);
                    }
                }
            }
            Expr::NamedExpr { target, value } => {
                Self::collect_names_expr(target, names);
                Self::collect_names_expr(value, names);
            }
        }
    }

    /// Find names assigned in a body (targets of =, for, function defs, etc.)
    fn collect_assigned_names(stmts: &[Stmt]) -> HashSet<String> {
        let mut assigned = HashSet::new();
        Self::collect_assigned_inner(stmts, &mut assigned);
        assigned
    }

    fn collect_assigned_inner(stmts: &[Stmt], assigned: &mut HashSet<String>) {
        for stmt in stmts {
            let stmt = Self::unwrap_located(stmt);
            match stmt {
                Stmt::Assign { targets, .. } => {
                    for t in targets {
                        Self::collect_assign_target_names(t, assigned);
                    }
                }
                Stmt::AugAssign { target, .. } => {
                    Self::collect_assign_target_names(target, assigned);
                }
                Stmt::AnnAssign {
                    target,
                    value: Some(_),
                    ..
                } => {
                    Self::collect_assign_target_names(target, assigned);
                }
                Stmt::For {
                    target,
                    body,
                    orelse,
                    ..
                } => {
                    Self::collect_assign_target_names(target, assigned);
                    Self::collect_assigned_inner(body, assigned);
                    Self::collect_assigned_inner(orelse, assigned);
                }
                Stmt::FunctionDef { name, .. } => {
                    assigned.insert(name.clone());
                }
                Stmt::ClassDef { name, .. } => {
                    assigned.insert(name.clone());
                }
                Stmt::If { body, orelse, .. } => {
                    Self::collect_assigned_inner(body, assigned);
                    Self::collect_assigned_inner(orelse, assigned);
                }
                Stmt::While { body, orelse, .. } => {
                    Self::collect_assigned_inner(body, assigned);
                    Self::collect_assigned_inner(orelse, assigned);
                }
                Stmt::With { items, body, .. } => {
                    for item in items {
                        if let Some(var) = &item.optional_vars {
                            Self::collect_assign_target_names(var, assigned);
                        }
                    }
                    Self::collect_assigned_inner(body, assigned);
                }
                Stmt::Match { cases, .. } => {
                    for case in cases {
                        Self::collect_assigned_inner(&case.body, assigned);
                    }
                }
                Stmt::Try {
                    body,
                    handlers,
                    handlers_star: _,
                    orelse,
                    finalbody,
                } => {
                    Self::collect_assigned_inner(body, assigned);
                    for h in handlers {
                        Self::collect_assigned_inner(&h.body, assigned);
                    }
                    Self::collect_assigned_inner(orelse, assigned);
                    Self::collect_assigned_inner(finalbody, assigned);
                }
                Stmt::Import(names_list) => {
                    for alias in names_list {
                        assigned.insert(alias.asname.clone().unwrap_or_else(|| alias.name.clone()));
                    }
                }
                Stmt::ImportFrom {
                    names: names_list, ..
                } => {
                    for alias in names_list {
                        assigned.insert(alias.asname.clone().unwrap_or_else(|| alias.name.clone()));
                    }
                }
                _ => {}
            }
        }
    }

    fn collect_assign_target_names(target: &Expr, assigned: &mut HashSet<String>) {
        match target {
            Expr::Name(n) => {
                assigned.insert(n.clone());
            }
            Expr::List(elts) | Expr::Tuple(elts) => {
                for e in elts {
                    Self::collect_assign_target_names(e, assigned);
                }
            }
            Expr::Starred(e) => Self::collect_assign_target_names(e, assigned),
            _ => {}
        }
    }

    /// Collect names referenced in the current function's own body (NOT nested function bodies).
    /// Names referenced anywhere in this function's own body — including
    /// inside `if`/`while`/`for`/`with`/`try`/`match` bodies, which don't
    /// introduce a new Python scope, so a name used only inside one of
    /// those (e.g. `def outer(x):\n def inner():\n  if True: return x`)
    /// must still be recognized as needing to come from an enclosing scope.
    /// Does NOT descend into nested FunctionDef/ClassDef bodies — those have
    /// their own scope and are handled separately (collect_nested_references).
    fn collect_own_referenced_names(stmts: &[Stmt]) -> HashSet<String> {
        let mut names = HashSet::new();
        Self::collect_own_referenced_names_inner(stmts, &mut names);
        names
    }

    fn collect_own_referenced_names_inner(stmts: &[Stmt], names: &mut HashSet<String>) {
        for stmt in stmts {
            let stmt = Self::unwrap_located(stmt);
            match stmt {
                Stmt::Located(..) => unreachable!("stmt already unwrapped via unwrap_located"),
                Stmt::Expr(expr) => Self::collect_names_expr(expr, names),
                Stmt::Return(Some(expr)) => Self::collect_names_expr(expr, names),
                Stmt::Return(None) | Stmt::Pass | Stmt::Break | Stmt::Continue => {}
                Stmt::Assign { targets, value } => {
                    Self::collect_names_expr(value, names);
                    for t in targets {
                        Self::collect_names_expr(t, names);
                    }
                }
                Stmt::AugAssign { target, value, .. } => {
                    Self::collect_names_expr(target, names);
                    Self::collect_names_expr(value, names);
                }
                Stmt::AnnAssign { target, value, .. } => {
                    Self::collect_names_expr(target, names);
                    if let Some(v) = value {
                        Self::collect_names_expr(v, names);
                    }
                }
                Stmt::If { test, body, orelse } => {
                    Self::collect_names_expr(test, names);
                    Self::collect_own_referenced_names_inner(body, names);
                    Self::collect_own_referenced_names_inner(orelse, names);
                }
                Stmt::While { test, body, orelse } => {
                    Self::collect_names_expr(test, names);
                    Self::collect_own_referenced_names_inner(body, names);
                    Self::collect_own_referenced_names_inner(orelse, names);
                }
                Stmt::For { target, iter, body, orelse, .. } => {
                    Self::collect_names_expr(target, names);
                    Self::collect_names_expr(iter, names);
                    Self::collect_own_referenced_names_inner(body, names);
                    Self::collect_own_referenced_names_inner(orelse, names);
                }
                Stmt::With { items, body, .. } => {
                    for item in items {
                        Self::collect_names_expr(&item.context_expr, names);
                        if let Some(var) = &item.optional_vars {
                            Self::collect_names_expr(var, names);
                        }
                    }
                    Self::collect_own_referenced_names_inner(body, names);
                }
                Stmt::Try { body, handlers, handlers_star, orelse, finalbody } => {
                    Self::collect_own_referenced_names_inner(body, names);
                    for h in handlers {
                        if let Some(t) = &h.typ {
                            Self::collect_names_expr(t, names);
                        }
                        Self::collect_own_referenced_names_inner(&h.body, names);
                    }
                    for h in handlers_star {
                        if let Some(t) = &h.typ {
                            Self::collect_names_expr(t, names);
                        }
                        Self::collect_own_referenced_names_inner(&h.body, names);
                    }
                    Self::collect_own_referenced_names_inner(orelse, names);
                    Self::collect_own_referenced_names_inner(finalbody, names);
                }
                Stmt::Raise { exc, cause } => {
                    if let Some(e) = exc {
                        Self::collect_names_expr(e, names);
                    }
                    if let Some(c) = cause {
                        Self::collect_names_expr(c, names);
                    }
                }
                Stmt::Assert { test, msg } => {
                    Self::collect_names_expr(test, names);
                    if let Some(m) = msg {
                        Self::collect_names_expr(m, names);
                    }
                }
                Stmt::Match { subject, cases } => {
                    Self::collect_names_expr(subject, names);
                    for case in cases {
                        if let Some(guard) = &case.guard {
                            Self::collect_names_expr(guard, names);
                        }
                        Self::collect_own_referenced_names_inner(&case.body, names);
                    }
                }
                Stmt::Delete(targets) => {
                    for t in targets {
                        Self::collect_names_expr(t, names);
                    }
                }
                Stmt::TypeAlias { value, .. } => Self::collect_names_expr(value, names),
                Stmt::FunctionDef { .. } | Stmt::ClassDef { .. } => {}
                Stmt::Import(_) | Stmt::ImportFrom { .. } | Stmt::Global(_) | Stmt::Nonlocal(_) => {}
            }
        }
    }

    /// Pre-analyze a function body to determine cell variables and free variables.
    /// Returns (cellvars, freevars)
    fn analyze_function(
        args: &[Arg],
        body: &[Stmt],
        global_names: &HashSet<String>,
        nonlocal_names: &HashSet<String>,
        enclosing_names: Option<&HashSet<String>>,
    ) -> (Vec<String>, Vec<String>) {
        // Find nonlocal declarations within this function's body
        let (body_globals, body_nonlocals) = Self::scan_global_nonlocal_decls(body);
        let mut effective_global = global_names.clone();
        let mut effective_nonlocal = nonlocal_names.clone();
        effective_global.extend(body_globals);
        effective_nonlocal.extend(body_nonlocals);

        // Collect all names assigned locally (including params)
        let mut local_names = Self::collect_assigned_names(body);
        for arg in args {
            local_names.insert(arg.arg.clone());
        }
        for n in &effective_nonlocal {
            local_names.remove(n);
        }
        for n in &effective_global {
            local_names.remove(n);
        }

        // Collect names referenced in THIS function's own body
        let own_refs = Self::collect_own_referenced_names(body);

        // Collect names referenced in nested function definitions
        let nested_refs = Self::collect_nested_references(
            body,
            &local_names,
            &effective_global,
            &effective_nonlocal,
        );

        // All names from outer scope = own_refs (not local) + nested_refs.
        // nested_refs may now include names needed by something nested two
        // or more levels down (relayed transitively through intervening
        // scopes) — only keep those that are either satisfiable by our own
        // locals (cellvar candidates, handled below) or genuinely available
        // from further out; anything else is a plain global/builtin and
        // must NOT be dragged in here.
        let mut all_outer_refs: HashSet<String> = nested_refs
            .iter()
            .filter(|n| local_names.contains(*n) || enclosing_names.map_or(true, |en| en.contains(*n)))
            .cloned()
            .collect();
        for name in &own_refs {
            if !local_names.contains(name)
                && !effective_global.contains(name)
                && enclosing_names.map_or(true, |en| en.contains(name))
            {
                all_outer_refs.insert(name.clone());
            }
        }

        // cell_vars = names a nested function needs that we must expose as
        // a cell — either because it's genuinely one of our own locals
        // (the original `local_names ∩ nested_refs` case), OR because it's
        // itself a free variable we only received from *our* enclosing
        // scope but a function nested inside *us* also needs it relayed
        // through (real code: `deprecate_posargs(deprecation_warning,
        // remappable_names, /)`'s nested `decorator` receives both as free
        // variables from `deprecate_posargs`, but `decorator`'s own nested
        // `remap_deprecated_args`/genexpr also reference them — so
        // `decorator` must re-expose them as cells, not just read them as
        // plain free variables). Missing this second case previously meant
        // such a name was only ever a free variable here, with no matching
        // cell — the nested function's own free-variable *index* (computed
        // against `cellvars.len() + position`, see `Expr::Name`'s LOAD_DEREF
        // emission) then silently pointed at a different, unrelated
        // variable once the enclosing function's real cellvars list (built
        // incrementally as nested closures compile) didn't match what this
        // upfront pass had promised. `all_outer_refs` (below) already holds
        // every name available from further out that could need this
        // treatment.
        let mut cell_vars: Vec<String> = local_names
            .intersection(&nested_refs)
            .filter(|n| !effective_global.contains(*n))
            .cloned()
            .collect();
        for name in all_outer_refs.intersection(&nested_refs) {
            if !local_names.contains(name) && !effective_global.contains(name) && !cell_vars.contains(name) {
                cell_vars.push(name.clone());
            }
        }
        cell_vars.sort();

        // free_vars = all_outer_refs - local_names (excluding global)
        let mut free_vars: Vec<String> = all_outer_refs
            .difference(&local_names)
            .filter(|n| !effective_global.contains(*n))
            .cloned()
            .collect();
        // Also include name referenced directly in this function that aren't local
        // but only if they exist in an enclosing function's scope (not module globals)
        for name in &own_refs {
            if !local_names.contains(name)
                && !free_vars.contains(name)
                && !effective_global.contains(name)
                && enclosing_names.map_or(true, |en| en.contains(name))
            {
                free_vars.push(name.clone());
            }
        }
        // Include explicit nonlocal declarations
        for n in &effective_nonlocal {
            if !free_vars.contains(n) && !effective_global.contains(n) {
                free_vars.push(n.clone());
            }
        }
        free_vars.sort();

        (cell_vars, free_vars)
    }

    /// Recursively find names referenced in nested function bodies that are NOT
    /// assigned within those nested functions.
    fn collect_nested_references(
        stmts: &[Stmt],
        local_names: &HashSet<String>,
        global_names: &HashSet<String>,
        nonlocal_names: &HashSet<String>,
    ) -> HashSet<String> {
        let mut refs = HashSet::new();
        Self::collect_nested_refs_inner(
            stmts,
            local_names,
            global_names,
            nonlocal_names,
            &mut refs,
        );
        refs
    }

    fn collect_nested_refs_inner(
        stmts: &[Stmt],
        local_names: &HashSet<String>,
        global_names: &HashSet<String>,
        nonlocal_names: &HashSet<String>,
        refs: &mut HashSet<String>,
    ) {
        for stmt in stmts {
            let stmt = Self::unwrap_located(stmt);
            match stmt {
                Stmt::FunctionDef { args, body, .. } => {
                    let (inner_globals, inner_nonlocals) = Self::scan_global_nonlocal_decls(body);
                    let mut inner_local = Self::collect_assigned_names(body);
                    for arg in args {
                        inner_local.insert(arg.arg.clone());
                    }
                    for n in &inner_nonlocals {
                        inner_local.remove(n);
                    }
                    for n in &inner_globals {
                        inner_local.remove(n);
                    }
                    // Names this nested function references directly that
                    // aren't its own locals — it needs these from an
                    // enclosing scope (either us, or further out still).
                    let own_refs = Self::collect_own_referenced_names(body);
                    for name in &own_refs {
                        if !inner_local.contains(name) && !inner_globals.contains(name) {
                            refs.insert(name.clone());
                        }
                    }
                    // Recurse: anything referenced by a function/class
                    // nested even deeper that isn't satisfied by THIS
                    // function's own locals also needs to come from further
                    // out than this function, i.e. from us or beyond.
                    let mut deeper = HashSet::new();
                    Self::collect_nested_refs_inner(
                        body,
                        &inner_local,
                        &inner_globals,
                        &inner_nonlocals,
                        &mut deeper,
                    );
                    for name in deeper {
                        if !inner_local.contains(&name) {
                            refs.insert(name);
                        }
                    }
                }
                // Class bodies are transparent for closure purposes: a method
                // defined inside a class inside a function can still close
                // over the function's locals (Python skips class scopes when
                // resolving enclosing references), so keep looking inside
                // using the same local_names as our caller.
                Stmt::ClassDef { body, .. } => {
                    Self::collect_nested_refs_inner(
                        body,
                        local_names,
                        global_names,
                        nonlocal_names,
                        refs,
                    );
                }
                // Control-flow statements are NOT their own scope — a nested
                // `def`/`class` inside an `if`/`while`/`for`/`try`/`with`
                // body is exactly as much a "nested function of the
                // enclosing function" as one written directly at its top
                // level (real code: `if iscoroutinefunction(func): async
                // def wrapper(...): ... else: def wrapper(...): ...`, a
                // completely ordinary sync/async-dispatching decorator
                // pattern). Previously these fell to the catch-all no-op
                // below, so a closure captured *only* by a conditionally-
                // defined nested function was never added to the enclosing
                // function's `cell_vars` during this upfront static pass —
                // it only got added later, lazily, while actually compiling
                // that nested function's closure-building code (see
                // compile_function's "Emit LOAD_CLOSURE" step) — by which
                // point any *other* free-variable reference already
                // compiled earlier in the enclosing function's own body
                // (e.g. the `if` condition itself) had already emitted a
                // `LOAD_DEREF` index computed against the *old, smaller*
                // `cellvars` list, silently going stale once `cellvars`
                // grew. (Cellvars sort before freevars in the combined
                // LOAD_DEREF index space, so any resulting off-by-one loads
                // the wrong variable outright — confirmed via a minimal
                // repro where an `if <closed-over free var>:` branch always
                // took the same path regardless of the free var's real
                // value, because index 0 pointed at a *cell* var instead.)
                Stmt::If { body, orelse, .. } => {
                    Self::collect_nested_refs_inner(body, local_names, global_names, nonlocal_names, refs);
                    Self::collect_nested_refs_inner(orelse, local_names, global_names, nonlocal_names, refs);
                }
                Stmt::While { body, orelse, .. } => {
                    Self::collect_nested_refs_inner(body, local_names, global_names, nonlocal_names, refs);
                    Self::collect_nested_refs_inner(orelse, local_names, global_names, nonlocal_names, refs);
                }
                Stmt::For { body, orelse, .. } => {
                    Self::collect_nested_refs_inner(body, local_names, global_names, nonlocal_names, refs);
                    Self::collect_nested_refs_inner(orelse, local_names, global_names, nonlocal_names, refs);
                }
                Stmt::With { body, .. } => {
                    Self::collect_nested_refs_inner(body, local_names, global_names, nonlocal_names, refs);
                }
                Stmt::Try { body, handlers, handlers_star, orelse, finalbody } => {
                    Self::collect_nested_refs_inner(body, local_names, global_names, nonlocal_names, refs);
                    for h in handlers {
                        Self::collect_nested_refs_inner(&h.body, local_names, global_names, nonlocal_names, refs);
                    }
                    for h in handlers_star {
                        Self::collect_nested_refs_inner(&h.body, local_names, global_names, nonlocal_names, refs);
                    }
                    Self::collect_nested_refs_inner(orelse, local_names, global_names, nonlocal_names, refs);
                    Self::collect_nested_refs_inner(finalbody, local_names, global_names, nonlocal_names, refs);
                }
                _ => {}
            }
        }
    }

    fn scan_global_nonlocal_decls(body: &[Stmt]) -> (HashSet<String>, HashSet<String>) {
        let mut globals = HashSet::new();
        let mut nonlocals = HashSet::new();
        for stmt in body {
            let stmt = Self::unwrap_located(stmt);
            match stmt {
                Stmt::Global(names) => {
                    for n in names {
                        globals.insert(n.clone());
                    }
                }
                Stmt::Nonlocal(names) => {
                    for n in names {
                        nonlocals.insert(n.clone());
                    }
                }
                _ => {}
            }
        }
        (globals, nonlocals)
    }

    // ---- Statement compilation ----

    /// Strips a `Stmt::Located` wrapper (added by the parser at each
    /// statement pushed into a block) down to the real statement. Statements
    /// synthesized by the compiler itself (e.g. multi-item `with` desugaring)
    /// are never wrapped and pass through unchanged.
    fn unwrap_located(stmt: &Stmt) -> &Stmt {
        match stmt {
            Stmt::Located(_, inner) => Self::unwrap_located(inner),
            _ => stmt,
        }
    }

    fn compile_stmts(&mut self, stmts: &[Stmt]) -> Result<(), String> {
        let mut first = true;
        for stmt in stmts {
            let stmt = if let Stmt::Located(line, inner) = stmt {
                self.set_line(*line);
                inner.as_ref()
            } else {
                stmt
            };
            if first {
                first = false;
                if matches!(stmt, Stmt::Match { .. }) {
                    self.emit(Opcode::NOP, 0);
                }
            }
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
                let (end_label, is_for, cleanup) = if let Some(loop_info) = self.loop_stack.last() {
                    (loop_info.end_label, loop_info.is_for, self.pending_cleanup.clone())
                } else {
                    return Err("'break' outside loop".to_string());
                };
                for entry in cleanup.iter().rev() {
                    match entry {
                        PendingCleanup::PopExcept => {
                            self.emit(Opcode::POP_EXCEPT, 0);
                        }
                        PendingCleanup::With(is_async) => {
                            let exit_name = if *is_async { "__aexit__" } else { "__exit__" };
                            let exit_name_idx = self.get_name_index(exit_name) as u32;
                            self.emit(Opcode::DUP_TOP, 0);
                            self.emit(Opcode::LOAD_ATTR, exit_name_idx);
                            let const_none = self.get_const_index(ConstValue::None) as u32;
                            for _ in 0..3 {
                                self.emit(Opcode::LOAD_CONST, const_none);
                            }
                            self.emit(Opcode::CALL, 3);
                            self.emit(Opcode::POP_TOP, 0);
                        }
                        PendingCleanup::Finally(_) => {
                            // Finally blocks are handled inline by the compiler,
                            // not through pending_cleanup walking — skip to avoid
                            // infinite recursion when continue/break is inside
                            // a finally block.
                        }
                    }
                }
                if is_for {
                    self.emit(Opcode::POP_TOP, 0);
                }
                self.emit_jump(Opcode::JUMP, end_label);
            }
            Stmt::Continue => {
                let (start_label, cleanup) = if let Some(loop_info) = self.loop_stack.last() {
                    (loop_info.start_label, self.pending_cleanup.clone())
                } else {
                    return Err("'continue' outside loop".to_string());
                };
                for entry in cleanup.iter().rev() {
                    match entry {
                        PendingCleanup::PopExcept => {
                            self.emit(Opcode::POP_EXCEPT, 0);
                        }
                        PendingCleanup::With(is_async) => {
                            let exit_name = if *is_async { "__aexit__" } else { "__exit__" };
                            let exit_name_idx = self.get_name_index(exit_name) as u32;
                            self.emit(Opcode::DUP_TOP, 0);
                            self.emit(Opcode::LOAD_ATTR, exit_name_idx);
                            let const_none = self.get_const_index(ConstValue::None) as u32;
                            for _ in 0..3 {
                                self.emit(Opcode::LOAD_CONST, const_none);
                            }
                            self.emit(Opcode::CALL, 3);
                            self.emit(Opcode::POP_TOP, 0);
                        }
                        PendingCleanup::Finally(_) => {
                        }
                    }
                }
                self.emit_backward_jump(start_label);
            }
            Stmt::Return(value) => {
                if let Some(expr) = value {
                    self.compile_expr(expr)?;
                } else {
                    let const_idx = self.get_const_index(ConstValue::None) as u32;
                    self.emit(Opcode::LOAD_CONST, const_idx);
                }
                // Returning from inside `with cm(): return x` must still run
                // cm.__exit__, and returning from inside `try: return x
                // finally: ...` must still run the `finally` body — CPython
                // inlines both at compile time rather than having the VM
                // unwind pending with/finally blocks on early return (this
                // VM's RETURN_VALUE doesn't do that either). Walk pending
                // cleanup entries innermost first (the order they were
                // pushed, reversed).
                for entry in self.pending_cleanup.clone().iter().rev() {
                    match entry {
                        PendingCleanup::With(is_async) => {
                            // At this point the stack is [..., cm_N, ...,
                            // cm_1, retval] (outermost with-block's manager
                            // deepest); swap the manager above retval, dup
                            // it, call __exit__(None,None,None), and discard
                            // both the call result and the manager, leaving
                            // retval on top again.
                            self.emit(Opcode::SWAP, 1);
                            self.emit(Opcode::DUP_TOP, 0);
                            let exit_name = if *is_async { "__aexit__" } else { "__exit__" };
                            let exit_name_idx = self.get_name_index(exit_name) as u32;
                            self.emit(Opcode::LOAD_ATTR, exit_name_idx);
                            let const_none = self.get_const_index(ConstValue::None) as u32;
                            for _ in 0..3 {
                                self.emit(Opcode::LOAD_CONST, const_none);
                            }
                            self.emit(Opcode::CALL, 3);
                            self.emit(Opcode::POP_TOP, 0);
                            self.emit(Opcode::POP_TOP, 0);
                        }
                        PendingCleanup::Finally(finalbody) => {
                            // Inline a fresh copy of the finally body right
                            // here (retval is safely parked below on the
                            // stack, untouched by these statements).
                            self.compile_stmts(finalbody)?;
                        }
                        PendingCleanup::PopExcept => {
                            // Stack here is [..., exc_info, retval] — retval
                            // (pushed first, by this Return's own codegen)
                            // sits ON TOP of the pushed exception info.
                            // `POP_EXCEPT` unconditionally pops whatever's
                            // on top of the stack (it has no notion of
                            // "the second slot down"), so popping it
                            // directly here would discard `retval` instead
                            // — swap the two first so `exc_info` ends up on
                            // top for `POP_EXCEPT` to remove, leaving
                            // `retval` on top again afterward.
                            self.emit(Opcode::SWAP, 1);
                            self.emit(Opcode::POP_EXCEPT, 0);
                        }
                    }
                }
                self.emit(Opcode::RETURN_VALUE, 0);
            }
            Stmt::Assign { targets, value } => {
                if targets.len() == 1 {
                    let target = &targets[0];
                    // For subscript assignment, compile obj and index before value
                    if let Expr::Subscript { value: obj, slice } = target {
                        self.compile_expr(obj)?;
                        self.compile_expr(slice)?;
                        self.compile_expr(value)?;
                        self.emit(Opcode::STORE_SUBSCR, 0);
                    } else {
                        self.compile_expr(value)?;
                        self.compile_assign_target(target)?;
                    }
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
                match &**target {
                    Expr::Subscript { value: obj, slice } => {
                        // For subscript augmented assignment like x[0] += 1:
                        self.compile_expr(obj)?;
                        self.compile_expr(slice)?;
                        self.emit(Opcode::COPY, 0);
                        self.emit(Opcode::COPY, 2);
                        self.emit(Opcode::SWAP, 1);
                        self.emit(Opcode::BINARY_OP, 13); // BINARY_SUBSCR
                        self.compile_expr(value)?;
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
                        self.emit(Opcode::BINARY_OP, bin_op + 100); // +100: in-place (see BINARY_OP's own doc comment)
                        self.emit(Opcode::STORE_SUBSCR, 0);
                    }
                    Expr::Attribute { value: obj, attr } => {
                        // For attribute augmented assignment like x.a += 1:
                        self.compile_expr(obj)?;
                        self.emit(Opcode::COPY, 0);
                        let attr_idx = self.get_name_index(attr) as u32;
                        self.emit(Opcode::LOAD_ATTR, attr_idx);
                        self.compile_expr(value)?;
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
                        self.emit(Opcode::BINARY_OP, bin_op + 100); // +100: in-place (see BINARY_OP's own doc comment)
                        // Stack here is already [obj, sum] (obj pushed once,
                        // duplicated for LOAD_ATTR, sum computed on top) —
                        // exactly what STORE_ATTR expects. No SWAP needed;
                        // unlike plain `x.attr = value` assignment (see
                        // compile_assign_target), which pushes obj AFTER the
                        // value and so must swap.
                        self.emit(Opcode::STORE_ATTR, attr_idx);
                    }
                    _ => {
                        self.compile_expr(target)?;
                        self.compile_expr(value)?;
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
                        self.emit(Opcode::BINARY_OP, bin_op + 100); // +100: in-place (see BINARY_OP's own doc comment)
                        self.compile_assign_target(target)?;
                    }
                }
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
                self.loop_stack.push(LoopInfo {
                    start_label,
                    end_label,
                    is_for: false,
                });
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
            Stmt::For {
                target,
                iter,
                body,
                orelse,
                is_async,
            } => {
                self.compile_expr(iter)?;
                if *is_async {
                    self.emit(Opcode::GET_AITER, 0);
                    self.emit(Opcode::GET_ANEXT, 0);
                } else {
                    self.emit(Opcode::GET_ITER, 0);
                }
                let start_label = self.new_label();
                let else_label = self.new_label();
                let end_label = self.new_label();
                self.loop_stack.push(LoopInfo {
                    start_label,
                    end_label,
                    is_for: true,
                });
                self.mark_label(start_label);
                self.emit_jump(if *is_async { Opcode::FOR_ITER } else { Opcode::FOR_ITER }, else_label);
                self.compile_assign_target(target)?;
                self.compile_stmts(body)?;
                self.emit_backward_jump(start_label);
                self.fix_label(else_label);
                if !orelse.is_empty() {
                    self.compile_stmts(orelse)?;
                }
                self.emit(Opcode::END_FOR, 0);
                self.fix_label(end_label);
                self.loop_stack.pop();
            }
            Stmt::FunctionDef {
                name,
                args,
                body,
                decorator_list,
                returns: _,
                is_async,
                ..
            } => {
                self.compile_function(name.clone(), args, body, *is_async)?;

                // Decorators apply bottom-up (closest to `def` first): `@a
                // @b def f` means `f = a(b(f))`, so `decorator_list` (given
                // in source/top-to-bottom order, `[a, b]`) must be walked in
                // reverse. This used to iterate forward — applying `a`
                // before `b` — which silently reordered any stacked
                // decorators where order is observable (e.g. `@classmethod
                // @functools.cache def f(cls): ...`, a common real-world
                // pattern, ended up building `cache(classmethod(f))`
                // instead of `classmethod(cache(f))`, wrapping a
                // `ClassMethod` object inside the cache instead of the
                // other way around). It also used to redundantly
                // pre-evaluate every decorator expression in a separate,
                // unrelated first pass whose pushed values were never
                // consumed — pure leftover stack garbage on every
                // decorated def, removed here along with the reordering.
                for decorator in decorator_list.iter().rev() {
                    self.compile_expr(decorator)?;
                    self.emit(Opcode::SWAP, 1);
                    self.emit(Opcode::CALL, 1);
                    // Result stays on stack
                }
                // Scope-aware storage (STORE_NAME/STORE_FAST/STORE_DEREF) —
                // see the matching fix on Stmt::ClassDef above for why
                // unconditional STORE_NAME breaks nested-function closures
                // over a helper function defined in the enclosing scope.
                self.compile_assign_target(&Expr::Name(name.clone()))?;
            }
            Stmt::ClassDef {
                name,
                bases,
                keywords: kw,
                body,
                decorator_list,
                ..
            } => {
                // Extract docstring from first statement if present
                let docstring = body.first().and_then(|s| {
                    if let Stmt::Expr(expr) = Self::unwrap_located(s) {
                        if let Expr::Constant(Constant::String(doc)) = expr.as_ref() {
                            Some(doc.clone())
                        } else {
                            None
                        }
                    } else {
                        None
                    }
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
                let kw_count = kw.len();
                for k in kw {
                    // Push keyword name as a string constant
                    let name_idx = self.get_const_index(
                        ConstValue::String(k.arg.clone().unwrap_or_default())
                    ) as u32;
                    self.emit(Opcode::LOAD_CONST, name_idx);
                    self.compile_expr(&k.value)?;
                }
                let call_arg = 3 | ((kw_count as u32) << 8);
                self.emit(Opcode::CALL, call_arg);
                // Set __doc__ on class if present
                if let Some(doc) = docstring {
                    self.emit(Opcode::DUP_TOP, 0);
                    let doc_idx = self.get_const_index(ConstValue::String(doc)) as u32;
                    self.emit(Opcode::LOAD_CONST, doc_idx);
                    let doc_attr_idx = self.get_name_index("__doc__") as u32;
                    self.emit(Opcode::STORE_ATTR, doc_attr_idx);
                }
                // Same bottom-up order as function decorators above (`@a @b
                // class C` means `C = a(b(C))`).
                for decorator in decorator_list.iter().rev() {
                    self.compile_expr(&decorator)?;
                    self.emit(Opcode::SWAP, 1);
                    self.emit(Opcode::CALL, 1);
                    // Decorated class stays on stack
                }
                // Use the same scope-aware storage logic as a regular
                // assignment (STORE_NAME/STORE_FAST/STORE_DEREF as
                // appropriate) — a class defined inside a function and
                // referenced by a nested closure needs STORE_DEREF into a
                // cell, exactly like any other name a closure captures.
                // Unconditional STORE_NAME here previously broke that case.
                self.compile_assign_target(&Expr::Name(name.clone()))?;
            }
            Stmt::Import(names) => {
                for alias in names {
                    let name_idx = self.get_name_index(&alias.name) as u32;
                    let const_none = self.get_const_index(ConstValue::None) as u32;
                    self.emit(Opcode::LOAD_CONST, const_none); // fromlist = None
                    self.emit(Opcode::LOAD_CONST, const_none); // level = 0 (None == 0)
                    self.emit(Opcode::IMPORT_NAME, name_idx);
                    if let Some(asname) = &alias.asname {
                        // `import a.b.c as x` must bind `x` to the LEAF
                        // submodule `a.b.c`, NOT the top-level package `a`
                        // that `IMPORT_NAME` returns when given an empty
                        // fromlist (matching real `__import__` semantics —
                        // fromlist empty => top package; fromlist non-empty
                        // => the named submodule). Previously stored
                        // whatever `IMPORT_NAME` pushed directly under
                        // `asname` with no attribute-chasing at all, so
                        // `import xml.etree.ElementTree as ET` bound `ET`
                        // to the `xml` PACKAGE instead of the `ElementTree`
                        // submodule (`ET.Element(...)` then raised
                        // `AttributeError: 'module' object has no attribute
                        // 'Element'` — confirmed via repro). Real CPython's
                        // own bytecode for this form does the identical
                        // walk: import once, then `LOAD_ATTR` down through
                        // each dotted component past the first.
                        for component in alias.name.split('.').skip(1) {
                            let attr_idx = self.get_name_index(component) as u32;
                            self.emit(Opcode::LOAD_ATTR, attr_idx);
                        }
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
            Stmt::ImportFrom {
                module,
                names,
                level,
            } => {
                let module_name = module.clone().unwrap_or_default();
                let name_idx = self.get_name_index(&module_name) as u32;
                // fromlist = tuple of imported names (needed for IMPORT_NAME semantics)
                let names_list: Vec<String> = names.iter().map(|a| a.name.clone()).collect();
                let fromlist_idx = self.get_const_index(ConstValue::Tuple(names_list)) as u32;
                self.emit(Opcode::LOAD_CONST, fromlist_idx);
                // level = number of dots (0 for absolute, 1+ for relative)
                let level_val = level.unwrap_or(0);
                let level_idx = self.get_const_index(ConstValue::Int(level_val.to_string())) as u32;
                self.emit(Opcode::LOAD_CONST, level_idx);
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
                // Pop the module reference left on stack after IMPORT_FROM loop
                self.emit(Opcode::POP_TOP, 0);
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
                        Expr::Subscript { value, slice } => {
                            self.compile_expr(value)?;
                            self.compile_expr(slice)?;
                            self.emit(Opcode::DELETE_SUBSCR, 0);
                        }
                        Expr::Attribute { value, attr } => {
                            self.compile_expr(value)?;
                            let name_idx = self.get_name_index(attr) as u32;
                            self.emit(Opcode::DELETE_ATTR, name_idx);
                        }
                        Expr::Name(name) => {
                            if self.scope == ScopeType::Module {
                                let idx = self.get_name_index(name) as u32;
                                self.emit(Opcode::DELETE_NAME, idx);
                            } else {
                                let idx = self.add_varname(name) as u32;
                                self.emit(Opcode::DELETE_FAST, idx);
                            }
                        }
                        Expr::Tuple(elts) | Expr::List(elts) => {
                            for e in elts {
                                match e {
                                    Expr::Subscript { value, slice } => {
                                        self.compile_expr(value)?;
                                        self.compile_expr(slice)?;
                                        self.emit(Opcode::DELETE_SUBSCR, 0);
                                    }
                                    Expr::Attribute { value, attr } => {
                                        self.compile_expr(value)?;
                                        let name_idx = self.get_name_index(attr) as u32;
                                        self.emit(Opcode::DELETE_ATTR, name_idx);
                                    }
                                    Expr::Name(name) => {
                                        if self.scope == ScopeType::Module {
                                            let idx = self.get_name_index(name) as u32;
                                            self.emit(Opcode::DELETE_NAME, idx);
                                        } else {
                                            let idx = self.add_varname(name) as u32;
                                            self.emit(Opcode::DELETE_FAST, idx);
                                        }
                                    }
                                    Expr::Tuple(inner) | Expr::List(inner) => {
                                        for ie in inner {
                                            // Recurse one level
                                            if let Expr::Name(n) = ie {
                                                if self.scope == ScopeType::Module {
                                                    let idx = self.get_name_index(n) as u32;
                                                    self.emit(Opcode::DELETE_NAME, idx);
                                                } else {
                                                    let idx = self.add_varname(n) as u32;
                                                    self.emit(Opcode::DELETE_FAST, idx);
                                                }
                                            }
                                        }
                                    }
                                    _ => return Err("cannot delete expression".to_string()),
                                }
                            }
                        }
                        _ => return Err("cannot delete expression".to_string()),
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
            Stmt::TypeAlias { name, value, .. } => {
                self.compile_expr(value)?;
                let name_idx = self.add_varname(name) as u32;
                self.emit(Opcode::STORE_FAST, name_idx);
            }
            Stmt::Located(line, inner) => {
                self.set_line(*line);
                self.compile_stmt(inner)?;
            }
            Stmt::Try {
                body,
                handlers,
                handlers_star,
                orelse,
                finalbody,
            } => {
                if !finalbody.is_empty() && handlers.is_empty() && handlers_star.is_empty() && orelse.is_empty() {
                    // Simple try/finally
                    let finally_label = self.new_label();
                    let end_label = self.new_label();
                    self.emit_jump(Opcode::SETUP_FINALLY, finally_label);
                    // Tracked so a `return` compiled inside `body` knows to
                    // inline a copy of `finalbody` first — see
                    // `pending_cleanup`'s doc comment.
                    self.pending_cleanup.push(PendingCleanup::Finally(finalbody.clone()));
                    let body_result = self.compile_stmts(body);
                    self.pending_cleanup.pop();
                    body_result?;
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
                    let after_orelse = self.new_label();
                    // Tracked so a `return` compiled inside `body`, any
                    // `handler.body`, or `orelse` knows to inline a copy of
                    // `finalbody` first — see `pending_cleanup`'s doc comment.
                    self.pending_cleanup.push(PendingCleanup::Finally(finalbody.clone()));
                    let body_result = self.compile_stmts(body);
                    body_result?;
                    self.emit(Opcode::POP_BLOCK, 0);
                    self.emit_jump(Opcode::JUMP, body_end);
                    self.fix_label(cleanup);
                    self.emit(Opcode::PUSH_EXC_INFO, 0);
                    // A `return` from inside any handler body below must
                    // `POP_EXCEPT` this pushed exception info before
                    // running any enclosing `with`'s cleanup — see
                    // `PendingCleanup::PopExcept`'s doc comment.
                    self.pending_cleanup.push(PendingCleanup::PopExcept);
                    // Compile regular except handlers
                    for handler in handlers {
                        if let Some(typ) = &handler.typ {
                            self.emit(Opcode::DUP_TOP, 0);
                            self.compile_expr(typ)?;
                            self.emit(Opcode::CHECK_EXC_MATCH, 0);
                            let next_handler = self.new_label();
                            self.emit_jump(Opcode::POP_JUMP_IF_FALSE, next_handler);
                            if let Some(name) = &handler.name {
                                // `except E as name:` consumes the pushed
                                // exception via this STORE — but the
                                // handler's shared `handler_done` epilogue
                                // (and, since this session's `PopExcept`
                                // fix, any `return` inside this body too)
                                // unconditionally `POP_EXCEPT`s whatever is
                                // on top of the stack, expecting to find
                                // the exception still there (as it is for
                                // a nameless `except E:`). DUP first so a
                                // copy survives the STORE for that later
                                // pop to consume — otherwise it silently
                                // pops whatever real value happens to sit
                                // below (e.g. an enclosing `with`'s own
                                // manager), corrupting it. Confirmed via
                                // the simplest repro: `with cm(): try:
                                // raise V() except V as e: pass` crashing
                                // with a stack underflow inside `cm`'s own
                                // `__exit__` several instructions later.
                                self.emit(Opcode::DUP_TOP, 0);
                                if self.scope == ScopeType::Module {
                                    let name_idx = self.get_name_index(name) as u32;
                                    self.emit(Opcode::STORE_NAME, name_idx);
                                } else {
                                    let idx = self.add_varname(name) as u32;
                                    self.emit(Opcode::STORE_FAST, idx);
                                }
                            }
                            self.compile_stmts(&handler.body)?;
                            self.emit_jump(Opcode::JUMP, handler_done);
                            self.fix_label(next_handler);
                        } else {
                            if let Some(name) = &handler.name {
                                // See the identical `DUP_TOP` comment above.
                                self.emit(Opcode::DUP_TOP, 0);
                                if self.scope == ScopeType::Module {
                                    let name_idx = self.get_name_index(name) as u32;
                                    self.emit(Opcode::STORE_NAME, name_idx);
                                } else {
                                    let idx = self.add_varname(name) as u32;
                                    self.emit(Opcode::STORE_FAST, idx);
                                }
                            }
                            self.compile_stmts(&handler.body)?;
                            self.emit_jump(Opcode::JUMP, handler_done);
                        }
                    }
                    // Compile except* handlers — split ExceptionGroups, fall through
                    for handler in handlers_star {
                        if let Some(typ) = &handler.typ {
                            self.emit(Opcode::DUP_TOP, 0);
                            self.compile_expr(typ)?;
                            self.emit(Opcode::CHECK_EXC_MATCH_STAR, 0);
                            let next_handler = self.new_label();
                            self.emit_jump(Opcode::POP_JUMP_IF_FALSE, next_handler);
                            if let Some(name) = &handler.name {
                                if self.scope == ScopeType::Module {
                                    let name_idx = self.get_name_index(name) as u32;
                                    self.emit(Opcode::STORE_NAME, name_idx);
                                } else {
                                    let idx = self.add_varname(name) as u32;
                                    self.emit(Opcode::STORE_FAST, idx);
                                }
                            } else {
                                self.emit(Opcode::POP_TOP, 0);
                            }
                            self.compile_stmts(&handler.body)?;
                            // Fall through to next handler (no JUMP to handler_done!)
                            self.fix_label(next_handler);
                        } else {
                            if let Some(name) = &handler.name {
                                if self.scope == ScopeType::Module {
                                    let name_idx = self.get_name_index(name) as u32;
                                    self.emit(Opcode::STORE_NAME, name_idx);
                                } else {
                                    let idx = self.add_varname(name) as u32;
                                    self.emit(Opcode::STORE_FAST, idx);
                                }
                            }
                            self.compile_stmts(&handler.body)?;
                        }
                    }
                    self.pending_cleanup.pop(); // PopExcept
                    self.emit(Opcode::RERAISE, 0);
                    self.fix_label(handler_done);
                    self.emit(Opcode::POP_EXCEPT, 0);
                    // Skip `orelse` (only the no-exception path runs it —
                    // see the sibling try/except-without-finally branch's
                    // comment for the exact bug this avoids) but still fall
                    // into the shared `finalbody` cleanup below, which must
                    // run regardless of whether an exception was handled.
                    self.emit_jump(Opcode::JUMP, after_orelse);
                    self.fix_label(body_end);
                    let orelse_result = if !orelse.is_empty() {
                        self.compile_stmts(orelse)
                    } else {
                        Ok(())
                    };
                    self.pending_cleanup.pop();
                    orelse_result?;
                    self.fix_label(after_orelse);
                    self.emit(Opcode::POP_BLOCK, 0);
                    self.compile_stmts(finalbody)?;
                    self.emit_jump(Opcode::JUMP, end_label);
                    self.fix_label(finally_label);
                    self.emit(Opcode::PUSH_EXC_INFO, 0);
                    self.compile_stmts(finalbody)?;
                    self.emit(Opcode::POP_EXCEPT, 0);
                    self.emit(Opcode::RERAISE, 0);
                    self.fix_label(end_label);
                } else if !handlers.is_empty() || !handlers_star.is_empty() {
                    let cleanup = self.new_label();
                    let _else_label = self.new_label();
                    let end_label = self.new_label();
                    self.emit_jump(Opcode::SETUP_FINALLY, cleanup);
                    let body_end = self.new_label();
                    let handler_done = self.new_label();
                    self.compile_stmts(body)?;
                    self.emit(Opcode::POP_BLOCK, 0);
                    self.emit_jump(Opcode::JUMP, body_end);
                    self.fix_label(cleanup);
                    self.emit(Opcode::PUSH_EXC_INFO, 0);
                    // See the identical marker in the try/except/finally
                    // branch above — `PendingCleanup::PopExcept`'s doc
                    // comment explains why this is needed.
                    self.pending_cleanup.push(PendingCleanup::PopExcept);
                    for handler in handlers {
                        if let Some(typ) = &handler.typ {
                            self.emit(Opcode::DUP_TOP, 0);
                            self.compile_expr(typ)?;
                            self.emit(Opcode::CHECK_EXC_MATCH, 0);
                            let next_handler = self.new_label();
                            self.emit_jump(Opcode::POP_JUMP_IF_FALSE, next_handler);
                            if let Some(name) = &handler.name {
                                // `except E as name:` consumes the pushed
                                // exception via this STORE — but the
                                // handler's shared `handler_done` epilogue
                                // (and, since this session's `PopExcept`
                                // fix, any `return` inside this body too)
                                // unconditionally `POP_EXCEPT`s whatever is
                                // on top of the stack, expecting to find
                                // the exception still there (as it is for
                                // a nameless `except E:`). DUP first so a
                                // copy survives the STORE for that later
                                // pop to consume — otherwise it silently
                                // pops whatever real value happens to sit
                                // below (e.g. an enclosing `with`'s own
                                // manager), corrupting it. Confirmed via
                                // the simplest repro: `with cm(): try:
                                // raise V() except V as e: pass` crashing
                                // with a stack underflow inside `cm`'s own
                                // `__exit__` several instructions later.
                                self.emit(Opcode::DUP_TOP, 0);
                                if self.scope == ScopeType::Module {
                                    let name_idx = self.get_name_index(name) as u32;
                                    self.emit(Opcode::STORE_NAME, name_idx);
                                } else {
                                    let idx = self.add_varname(name) as u32;
                                    self.emit(Opcode::STORE_FAST, idx);
                                }
                            }
                            self.compile_stmts(&handler.body)?;
                            self.emit_jump(Opcode::JUMP, handler_done);
                            self.fix_label(next_handler);
                        } else {
                            if let Some(name) = &handler.name {
                                // See the identical `DUP_TOP` comment above.
                                self.emit(Opcode::DUP_TOP, 0);
                                if self.scope == ScopeType::Module {
                                    let name_idx = self.get_name_index(name) as u32;
                                    self.emit(Opcode::STORE_NAME, name_idx);
                                } else {
                                    let idx = self.add_varname(name) as u32;
                                    self.emit(Opcode::STORE_FAST, idx);
                                }
                            }
                            self.compile_stmts(&handler.body)?;
                            self.emit_jump(Opcode::JUMP, handler_done);
                        }
                    }
                    // Compile except* handlers — split ExceptionGroups, fall through
                    for handler in handlers_star {
                        if let Some(typ) = &handler.typ {
                            self.emit(Opcode::DUP_TOP, 0);
                            self.compile_expr(typ)?;
                            self.emit(Opcode::CHECK_EXC_MATCH_STAR, 0);
                            let next_handler = self.new_label();
                            self.emit_jump(Opcode::POP_JUMP_IF_FALSE, next_handler);
                            if let Some(name) = &handler.name {
                                if self.scope == ScopeType::Module {
                                    let name_idx = self.get_name_index(name) as u32;
                                    self.emit(Opcode::STORE_NAME, name_idx);
                                } else {
                                    let idx = self.add_varname(name) as u32;
                                    self.emit(Opcode::STORE_FAST, idx);
                                }
                            } else {
                                self.emit(Opcode::POP_TOP, 0);
                            }
                            self.compile_stmts(&handler.body)?;
                            // Fall through to next handler (no JUMP to handler_done!)
                            self.fix_label(next_handler);
                        } else {
                            if let Some(name) = &handler.name {
                                if self.scope == ScopeType::Module {
                                    let name_idx = self.get_name_index(name) as u32;
                                    self.emit(Opcode::STORE_NAME, name_idx);
                                } else {
                                    let idx = self.add_varname(name) as u32;
                                    self.emit(Opcode::STORE_FAST, idx);
                                }
                            }
                            self.compile_stmts(&handler.body)?;
                        }
                    }
                    self.pending_cleanup.pop(); // PopExcept
                    self.emit(Opcode::RERAISE, 0);
                    self.fix_label(handler_done);
                    self.emit(Opcode::POP_EXCEPT, 0);
                    // A handled exception must NOT fall through into
                    // `orelse` — real Python's `try/except/else` only runs
                    // `else` when the `try` body completed with no
                    // exception at all. Without this jump, `orelse` ran
                    // unconditionally after ANY handler finished too (the
                    // `Opcode::ELSE` emitted below is purely a no-op
                    // marker, not an actual guard) — confirmed via the
                    // simplest possible repro (`try: raise ValueError()
                    // except ValueError: pass else: print("bug")` printing
                    // "bug"), and the real trigger that surfaced it:
                    // `unittest`'s own `case.py`'s `_addDuration`, whose
                    // `except AttributeError: warn(...) else:
                    // addDuration(...)` ran BOTH branches, hitting
                    // `NameError: addDuration referenced before assignment`
                    // whenever a `TestResult` legitimately had no
                    // `addDuration` method.
                    self.emit_jump(Opcode::JUMP, end_label);
                    self.fix_label(body_end);
                    if !orelse.is_empty() {
                        self.emit(Opcode::ELSE, 0);
                        self.compile_stmts(orelse)?;
                    }
                    self.fix_label(end_label);
                } else {
                    self.compile_stmts(body)?;
                }
            }
            Stmt::Assert { test, msg } => {
                let assertion_error_idx =
                    self.get_name_index("AssertionError") as u32;
                self.emit(Opcode::LOAD_GLOBAL, assertion_error_idx);
                self.compile_expr(test)?;
                let ok_label = self.new_label();
                self.emit_jump(Opcode::POP_JUMP_IF_TRUE, ok_label);
                let mut args = 0;
                if let Some(msg) = msg {
                    self.compile_expr(msg)?;
                    args = 1;
                }
                self.emit(Opcode::CALL, args);
                self.emit(Opcode::RAISE_VARARGS, 1);
                self.fix_label(ok_label);
                self.emit(Opcode::POP_TOP, 0);
            }
            Stmt::With { items, body, is_async } => {
                // `with a, b, c: BODY` is exactly equivalent to nested
                // single-item withs (`with a:\n with b:\n  with c:\n
                // BODY`) — CPython desugars it the same way. Previously this
                // only ever compiled `SETUP_WITH` for every item up front
                // and then fell to a bare `self.compile_stmts(body)` with
                // ZERO cleanup for anything but the single-item case — a
                // multi-manager `with` never called `__exit__` at all, not
                // even on the ordinary non-exceptional path (confirmed via
                // a minimal repro: no "exit"-side effect ever ran). Fixed by
                // actually desugaring into nested single-item `Stmt::With`
                // nodes and recursing, so every level reuses the one
                // correct, already-verified single-item try/finally
                // machinery below instead of a second, never-implemented
                // multi-item path.
                if items.len() > 1 {
                    let mut rest = items.clone();
                    let first = rest.remove(0);
                    let inner = Stmt::With { items: rest, body: body.clone(), is_async: *is_async };
                    self.compile_stmt(&Stmt::With { items: vec![first], body: vec![inner], is_async: *is_async })?;
                    return Ok(());
                }
                for (_i, item) in items.iter().enumerate() {
                    self.compile_expr(&item.context_expr)?;
                    if *is_async {
                        self.emit(Opcode::BEFORE_ASYNC_WITH, 0);
                    } else {
                        self.emit(Opcode::SETUP_WITH, 0);
                    }
                    if let Some(var) = &item.optional_vars {
                        self.compile_assign_target(var)?;
                    } else {
                        self.emit(Opcode::POP_TOP, 0);
                    }
                }
                if items.len() == 1 {
                    // Use try/finally to ensure __exit__/__aexit__ is called on exception
                    let finally_label = self.new_label();
                    let end_label = self.new_label();
                    self.emit_jump(Opcode::SETUP_FINALLY, finally_label);
                    // Tracked so a `return` compiled inside body knows to
                    // inline an __exit__ call for this with-block first —
                    // see `pending_cleanup`'s doc comment.
                    self.pending_cleanup.push(PendingCleanup::With(*is_async));
                    let with_result = self.compile_stmts(body);
                    self.pending_cleanup.pop();
                    with_result?;
                    self.emit(Opcode::POP_BLOCK, 0);
                    // Manager is still on the stack from SETUP_WITH
                    self.emit(Opcode::DUP_TOP, 0);
                    let exit_name = if *is_async { "__aexit__" } else { "__exit__" };
                    let exit_name_idx = self.get_name_index(exit_name) as u32;
                    self.emit(Opcode::LOAD_ATTR, exit_name_idx);
                    let const_none = self.get_const_index(ConstValue::None) as u32;
                    for _ in 0..3 {
                        self.emit(Opcode::LOAD_CONST, const_none);
                    }
                    self.emit(Opcode::CALL, 3);
                    self.emit(Opcode::POP_TOP, 0); // discard __exit__'s return value
                    self.emit(Opcode::POP_TOP, 0); // discard the manager itself (DUP_TOP'd above, never otherwise consumed)
                    self.emit_jump(Opcode::JUMP, end_label);
                    self.fix_label(finally_label);
                    self.emit(Opcode::PUSH_EXC_INFO, 0);
                    // Stack here is [manager, exception] — handle_exception()
                    // truncated to the depth right after SETUP_WITH (which
                    // left just the manager) and then pushed the exception
                    // object on top. WITH_EXIT wants the opposite order
                    // ([exception, manager], manager on top so it can pop
                    // it) — this used to DUP_TOP here, which duplicates
                    // whatever's actually on top (the exception, not the
                    // manager), so WITH_EXIT would pop a copy of the
                    // exception and treat IT as the context manager. Swap
                    // the two into the order WITH_EXIT actually expects.
                    self.emit(Opcode::SWAP, 1);
                    self.emit(Opcode::WITH_EXIT, 0);
                    // Stack: [..., exception_obj, exit_result]. Per PEP 343, a
                    // truthy return from __exit__ suppresses the exception —
                    // this was previously ignored (always RERAISE), silently
                    // breaking contextlib.suppress and any custom
                    // exception-swallowing context manager.
                    let suppress_label = self.new_label();
                    self.emit_jump(Opcode::POP_JUMP_IF_TRUE, suppress_label);
                    self.emit(Opcode::POP_TOP, 0); // discard exception_obj
                    self.emit(Opcode::RERAISE, 0);
                    self.fix_label(suppress_label);
                    self.emit(Opcode::POP_TOP, 0); // discard exception_obj — suppressed
                    self.fix_label(end_label);
                } else {
                    self.compile_stmts(body)?;
                }
            }
            Stmt::AnnAssign {
                target,
                annotation,
                value,
            } => {
                // Module/class-body annotations populate a real
                // `__annotations__` dict in that scope's namespace — this
                // used to be silently discarded entirely (`annotation: _`),
                // which broke `cls.__annotations__`/`__annotations__` for
                // EVERY class and module, a foundational gap for any code
                // introspecting types (dataclasses, typing.get_type_hints,
                // NamedTuple/TypedDict-style patterns). Function-scope
                // annotations are left as before (evaluated only via the
                // value-assignment path below, nothing stored) — real
                // CPython doesn't expose local variable annotations anywhere
                // either.
                if let Expr::Name(name) = target.as_ref() {
                    if self.scope == ScopeType::Module || self.scope == ScopeType::ClassBody {
                        if !self.annotations_initialized {
                            self.emit(Opcode::BUILD_MAP, 0);
                            let ann_idx = self.get_name_index("__annotations__") as u32;
                            self.emit(Opcode::STORE_NAME, ann_idx);
                            self.annotations_initialized = true;
                        }
                        let ann_idx = self.get_name_index("__annotations__") as u32;
                        self.emit(Opcode::LOAD_NAME, ann_idx);
                        let name_const = self.get_const_index(ConstValue::String(name.clone())) as u32;
                        self.emit(Opcode::LOAD_CONST, name_const);
                        // Evaluating the annotation eagerly (this
                        // interpreter doesn't implement PEP 649/749's real
                        // lazy-annotation deferral) can raise NameError for
                        // legitimate, common real-world code: CPython 3.14's
                        // own stdlib now routinely uses type-checking-only
                        // names in annotations relying on the new default
                        // lazy evaluation (e.g. `_colorize.py`'s
                        // `__dataclass_fields__: ClassVar[...]`, where
                        // `ClassVar` is only imported under `if False:`).
                        // Catch NameError specifically here (nothing else)
                        // and fall back to None — a pragmatic middle ground:
                        // real forward-reference-style annotations don't
                        // crash class definition, while annotations that
                        // fail for an unrelated reason (TypeError,
                        // ZeroDivisionError, ...) still propagate normally.
                        let tmp = "__annotation_tmp__".to_string();
                        let try_stmt = Stmt::Try {
                            body: vec![Stmt::Assign {
                                targets: vec![Expr::Name(tmp.clone())],
                                value: Box::new((**annotation).clone()),
                            }],
                            handlers: vec![ExceptHandler {
                                typ: Some(Box::new(Expr::Name("NameError".to_string()))),
                                name: None,
                                body: vec![Stmt::Assign {
                                    targets: vec![Expr::Name(tmp.clone())],
                                    value: Box::new(Expr::Constant(Constant::None)),
                                }],
                            }],
                            handlers_star: vec![],
                            orelse: vec![],
                            finalbody: vec![],
                        };
                        self.compile_stmt(&try_stmt)?;
                        self.compile_expr(&Expr::Name(tmp))?;
                        self.emit(Opcode::STORE_SUBSCR, 0);
                    }
                }
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
                        Pattern::MatchSequence(patterns) => {
                            // MatchSequence: check length and match elements
                            let star_index = patterns.iter().position(|p| matches!(p, Pattern::MatchStar { .. }));
                            self.emit(Opcode::DUP_TOP, 0);
                            // Get length of subject
                            let len_name_idx = self.get_name_index("len") as u32;
                            self.emit(Opcode::LOAD_GLOBAL, len_name_idx);
                            self.emit(Opcode::SWAP, 1);
                            self.emit(Opcode::CALL, 1);
                            if let Some(si) = star_index {
                                // With MatchStar: require len(subject) >= patterns.len() - 1
                                let min_len = patterns.len() - 1;
                                let length_const = self.get_const_index(ConstValue::Int(min_len.to_string())) as u32;
                                self.emit(Opcode::LOAD_CONST, length_const);
                                self.emit(Opcode::COMPARE_OP, 5); // >=
                                self.emit_jump(Opcode::POP_JUMP_IF_FALSE, next_case);
                                // Use UNPACK_EX to extract elements into natural order on the stack
                                let before = si;
                                let after = patterns.len() - si - 1;
                                let unpack_arg = ((before << 8) | after) as u32;
                                self.emit(Opcode::DUP_TOP, 0);
                                self.emit(Opcode::UNPACK_EX, unpack_arg);
                                // Process each pattern in order; stack now holds elements in pattern order
                                for pat in patterns {
                                    match pat {
                                        Pattern::MatchValue(val) => {
                                            self.compile_expr(val)?;
                                            self.emit(Opcode::COMPARE_OP, 2); // ==
                                            self.emit_jump(Opcode::POP_JUMP_IF_FALSE, next_case);
                                        }
                                        Pattern::MatchAs { name: Some(n), .. } => {
                                            let idx = self.add_varname(n) as u32;
                                            self.emit(Opcode::STORE_FAST, idx);
                                        }
                                        Pattern::MatchAs { name: None, .. } => {
                                            self.emit(Opcode::POP_TOP, 0);
                                        }
                                        Pattern::MatchSingleton(s) => {
                                            let const_idx = self.get_const_index(match s.as_str() {
                                                "None" => ConstValue::None,
                                                "True" => ConstValue::Bool(true),
                                                "False" => ConstValue::Bool(false),
                                                _ => ConstValue::String(s.clone()),
                                            }) as u32;
                                            self.emit(Opcode::LOAD_CONST, const_idx);
                                            self.emit(Opcode::COMPARE_OP, 8); // IS
                                            self.emit_jump(Opcode::POP_JUMP_IF_FALSE, next_case);
                                        }
                                        Pattern::MatchStar { name } => {
                                            match name {
                                                Some(n) => {
                                                    let idx = self.add_varname(n) as u32;
                                                    self.emit(Opcode::STORE_FAST, idx);
                                                }
                                                None => {
                                                    self.emit(Opcode::POP_TOP, 0);
                                                }
                                            }
                                        }
                                        _ => { self.emit(Opcode::POP_TOP, 0); }
                                    }
                                }
                            } else {
                                // No MatchStar: exact length check + sequential extraction
                                let length_const = self.get_const_index(ConstValue::Int(patterns.len().to_string())) as u32;
                                self.emit(Opcode::LOAD_CONST, length_const);
                                self.emit(Opcode::COMPARE_OP, 2); // ==
                                self.emit_jump(Opcode::POP_JUMP_IF_FALSE, next_case);
                                for (i, pat) in patterns.iter().enumerate() {
                                    let idx_const = self.get_const_index(ConstValue::Int(i.to_string())) as u32;
                                    self.emit(Opcode::DUP_TOP, 0);
                                    self.emit(Opcode::LOAD_CONST, idx_const);
                                    self.emit(Opcode::BINARY_OP, 13); // BINARY_SUBSCR
                                    match pat {
                                        Pattern::MatchValue(val) => {
                                            self.compile_expr(val)?;
                                            self.emit(Opcode::COMPARE_OP, 2); // ==
                                            self.emit_jump(Opcode::POP_JUMP_IF_FALSE, next_case);
                                        }
                                        Pattern::MatchAs { name: Some(n), .. } => {
                                            let idx = self.add_varname(n) as u32;
                                            self.emit(Opcode::STORE_FAST, idx);
                                        }
                                        Pattern::MatchAs { name: None, .. } => {
                                            self.emit(Opcode::POP_TOP, 0);
                                        }
                                        Pattern::MatchSingleton(s) => {
                                            let const_idx = self.get_const_index(match s.as_str() {
                                                "None" => ConstValue::None,
                                                "True" => ConstValue::Bool(true),
                                                "False" => ConstValue::Bool(false),
                                                _ => ConstValue::String(s.clone()),
                                            }) as u32;
                                            self.emit(Opcode::LOAD_CONST, const_idx);
                                            self.emit(Opcode::COMPARE_OP, 8); // IS
                                            self.emit_jump(Opcode::POP_JUMP_IF_FALSE, next_case);
                                        }
                                        _ => { self.emit(Opcode::POP_TOP, 0); }
                                    }
                                }
                            }
                            if let Some(guard) = &case.guard {
                                self.compile_expr(guard)?;
                                self.emit_jump(Opcode::POP_JUMP_IF_FALSE, next_case);
                            }
                        }
                        Pattern::MatchMapping { keys, rest } => {
                            // MatchMapping: check key presence and match values
                            // keys are interleaved: [key1_pat, val1_pat, key2_pat, val2_pat, ...]
                            for chunk in keys.chunks(2) {
                                if let [key_pat, val_pat] = chunk {
                                    // Key must be a literal value
                                    let key_expr = match key_pat {
                                        Pattern::MatchValue(expr) => expr,
                                        _ => return Err("Mapping pattern keys must be literal values".to_string()),
                                    };
                                    // Check key in subject
                                    self.emit(Opcode::DUP_TOP, 0);
                                    self.compile_expr(key_expr)?;
                                    self.emit(Opcode::CONTAINS_OP, 0);
                                    self.emit_jump(Opcode::POP_JUMP_IF_FALSE, next_case);
                                    // Get value: subject[key]
                                    self.emit(Opcode::DUP_TOP, 0);
                                    self.compile_expr(key_expr)?;
                                    self.emit(Opcode::BINARY_OP, 13); // BINARY_SUBSCR
                                    // Match value against pattern
                                    match val_pat {
                                        Pattern::MatchValue(val) => {
                                            self.compile_expr(val)?;
                                            self.emit(Opcode::COMPARE_OP, 2); // ==
                                            self.emit_jump(Opcode::POP_JUMP_IF_FALSE, next_case);
                                        }
                                        Pattern::MatchAs { name: Some(n), .. } => {
                                            let idx = self.add_varname(n) as u32;
                                            self.emit(Opcode::STORE_FAST, idx);
                                        }
                                        Pattern::MatchAs { name: None, .. } => {
                                            self.emit(Opcode::POP_TOP, 0);
                                        }
                                        Pattern::MatchSingleton(s) => {
                                            let const_idx = self.get_const_index(match s.as_str() {
                                                "None" => ConstValue::None,
                                                "True" => ConstValue::Bool(true),
                                                "False" => ConstValue::Bool(false),
                                                _ => ConstValue::String(s.clone()),
                                            }) as u32;
                                            self.emit(Opcode::LOAD_CONST, const_idx);
                                            self.emit(Opcode::COMPARE_OP, 8); // IS
                                            self.emit_jump(Opcode::POP_JUMP_IF_FALSE, next_case);
                                        }
                                        Pattern::MatchSequence(..) => {
                                            self.emit(Opcode::DUP_TOP, 0);
                                            let list_idx = self.get_const_index(ConstValue::String("list".to_string())) as u32;
                                            self.emit(Opcode::LOAD_CONST, list_idx);
                                            self.emit(Opcode::CONTAINS_OP, 1);
                                            let seq_ok = self.new_label();
                                            self.emit_jump(Opcode::POP_JUMP_IF_TRUE, seq_ok);
                                            let tuple_idx = self.get_const_index(ConstValue::String("tuple".to_string())) as u32;
                                            self.emit(Opcode::LOAD_CONST, tuple_idx);
                                            self.emit(Opcode::CONTAINS_OP, 1);
                                            self.emit_jump(Opcode::POP_JUMP_IF_FALSE, next_case);
                                            self.emit_label(seq_ok);
                                            self.emit(Opcode::POP_TOP, 0);
                                        }
                                        Pattern::MatchMapping { .. } => {
                                            self.emit(Opcode::DUP_TOP, 0);
                                            let dict_idx = self.get_const_index(ConstValue::String("dict".to_string())) as u32;
                                            self.emit(Opcode::LOAD_CONST, dict_idx);
                                            self.emit(Opcode::CONTAINS_OP, 1);
                                            self.emit_jump(Opcode::POP_JUMP_IF_FALSE, next_case);
                                            self.emit(Opcode::POP_TOP, 0);
                                        }
                                        Pattern::MatchOr(subpatterns) => {
                                            let or_matched = self.new_label();
                                            for subpat in subpatterns {
                                                self.emit(Opcode::DUP_TOP, 0);
                                                let try_next = self.new_label();
                                                match subpat {
                                                    Pattern::MatchValue(val) => {
                                                        self.compile_expr(val)?;
                                                        self.emit(Opcode::COMPARE_OP, 2);
                                                        self.emit_jump(Opcode::POP_JUMP_IF_FALSE, try_next);
                                                        self.emit_jump(Opcode::JUMP, or_matched);
                                                    }
                                                    Pattern::MatchAs { name: Some(n), .. } => {
                                                        let idx = self.add_varname(n) as u32;
                                                        self.emit(Opcode::STORE_FAST, idx);
                                                        self.emit_jump(Opcode::JUMP, or_matched);
                                                    }
                                                    _ => { self.emit(Opcode::POP_TOP, 0); }
                                                }
                                                self.emit_label(try_next);
                                            }
                                            self.emit_jump(Opcode::JUMP, next_case);
                                            self.emit_label(or_matched);
                                            self.emit(Opcode::POP_TOP, 0);
                                        }
                                        _ => return Err("Mapping sub-pattern not supported".to_string()),
                                    }
                                }
                            }
                            // Handle **rest capture
                            if let Some(rest_name) = rest {
                                let idx = self.add_varname(rest_name) as u32;
                                self.emit(Opcode::DUP_TOP, 0);
                                self.emit(Opcode::STORE_FAST, idx);
                            }
                            if let Some(guard) = &case.guard {
                                self.compile_expr(guard)?;
                                self.emit_jump(Opcode::POP_JUMP_IF_FALSE, next_case);
                            }
                        }
                        Pattern::MatchStar { name } => {
                            // MatchStar: capture remaining elements or discard
                            self.emit(Opcode::DUP_TOP, 0);
                            match name {
                                Some(n) => {
                                    let idx = self.add_varname(n) as u32;
                                    self.emit(Opcode::STORE_FAST, idx);
                                }
                                None => {
                                    self.emit(Opcode::POP_TOP, 0);
                                }
                            }
                            if let Some(guard) = &case.guard {
                                self.compile_expr(guard)?;
                                self.emit_jump(Opcode::POP_JUMP_IF_FALSE, next_case);
                            }
                        }
                        Pattern::MatchOr(subpatterns) => {
                            let or_matched = self.new_label();
                            for pat in subpatterns {
                                match pat {
                                    Pattern::MatchAs { name: Some(n), .. } => {
                                        self.emit(Opcode::DUP_TOP, 0);
                                        let idx = self.add_varname(n) as u32;
                                        self.emit(Opcode::STORE_FAST, idx);
                                        self.emit_jump(Opcode::JUMP, or_matched);
                                    }
                                    Pattern::MatchAs { name: None, .. } => {
                                        // Wildcard: always matches
                                        self.emit(Opcode::DUP_TOP, 0);
                                        self.emit(Opcode::POP_TOP, 0);
                                        self.emit_jump(Opcode::JUMP, or_matched);
                                    }
                                    Pattern::MatchValue(val) => {
                                        self.emit(Opcode::DUP_TOP, 0);
                                        let try_next = self.new_label();
                                        self.compile_expr(val)?;
                                        self.emit(Opcode::COMPARE_OP, 2); // ==
                                        self.emit_jump(Opcode::POP_JUMP_IF_FALSE, try_next);
                                        self.emit_jump(Opcode::JUMP, or_matched);
                                        self.fix_label(try_next);
                                    }
                                    Pattern::MatchSingleton(s) => {
                                        self.emit(Opcode::DUP_TOP, 0);
                                        let try_next = self.new_label();
                                        let const_idx = self.get_const_index(match s.as_str() {
                                            "None" => ConstValue::None,
                                            "True" => ConstValue::Bool(true),
                                            "False" => ConstValue::Bool(false),
                                            _ => ConstValue::String(s.clone()),
                                        }) as u32;
                                        self.emit(Opcode::LOAD_CONST, const_idx);
                                        self.emit(Opcode::COMPARE_OP, 8); // IS
                                        self.emit_jump(Opcode::POP_JUMP_IF_FALSE, try_next);
                                        self.emit_jump(Opcode::JUMP, or_matched);
                                        self.fix_label(try_next);
                                    }
                                    Pattern::MatchClass { cls, patterns, kwd_attrs, kwd_patterns } => {
                                        // Same isinstance-then-subpattern check as the
                                        // plain (non-Or) MatchClass arm below, but a
                                        // failure falls through to the next alternative
                                        // (try_next) instead of the next case.
                                        self.emit(Opcode::DUP_TOP, 0);
                                        let try_next = self.new_label();
                                        let isinstance_idx = self.get_name_index("isinstance") as u32;
                                        self.emit(Opcode::LOAD_GLOBAL, isinstance_idx);
                                        self.emit(Opcode::SWAP, 1);
                                        self.compile_expr(cls)?;
                                        self.emit(Opcode::CALL, 2);
                                        self.emit_jump(Opcode::POP_JUMP_IF_FALSE, try_next);
                                        for sub in patterns {
                                            self.emit(Opcode::DUP_TOP, 0);
                                            match sub {
                                                Pattern::MatchValue(val) => {
                                                    self.compile_expr(val)?;
                                                    self.emit(Opcode::COMPARE_OP, 2);
                                                    self.emit_jump(Opcode::POP_JUMP_IF_FALSE, try_next);
                                                }
                                                Pattern::MatchAs { name: Some(n), .. } => {
                                                    let idx = self.add_varname(n) as u32;
                                                    self.emit(Opcode::STORE_FAST, idx);
                                                }
                                                Pattern::MatchAs { name: None, .. } => {
                                                    self.emit(Opcode::POP_TOP, 0);
                                                }
                                                _ => { self.emit(Opcode::POP_TOP, 0); }
                                            }
                                        }
                                        for (kwd_attr, kwd_pat) in kwd_attrs.iter().zip(kwd_patterns.iter()) {
                                            self.emit(Opcode::DUP_TOP, 0);
                                            let attr_idx = self.get_name_index(kwd_attr) as u32;
                                            self.emit(Opcode::LOAD_ATTR, attr_idx);
                                            match kwd_pat {
                                                Pattern::MatchValue(val) => {
                                                    self.compile_expr(val)?;
                                                    self.emit(Opcode::COMPARE_OP, 2);
                                                    self.emit_jump(Opcode::POP_JUMP_IF_FALSE, try_next);
                                                }
                                                Pattern::MatchAs { name: Some(n), .. } => {
                                                    let idx = self.add_varname(n) as u32;
                                                    self.emit(Opcode::STORE_FAST, idx);
                                                }
                                                Pattern::MatchAs { name: None, .. } => {
                                                    self.emit(Opcode::POP_TOP, 0);
                                                }
                                                _ => { self.emit(Opcode::POP_TOP, 0); }
                                            }
                                        }
                                        self.emit_jump(Opcode::JUMP, or_matched);
                                        self.fix_label(try_next);
                                    }
                                    _ => {
                                        // Unsupported subpattern in OR — just pop and try next
                                        self.emit(Opcode::POP_TOP, 0);
                                    }
                                }
                            }
                            // All alternatives failed
                            self.emit_jump(Opcode::JUMP, next_case);
                            self.fix_label(or_matched);
                            // Check guard if present
                            if let Some(guard) = &case.guard {
                                self.emit(Opcode::DUP_TOP, 0);
                                self.compile_expr(guard)?;
                                self.emit_jump(Opcode::POP_JUMP_IF_FALSE, next_case);
                            }
                            // Fall through to POP_TOP subject, body, JUMP end_label
                        }
                        Pattern::MatchClass { cls, patterns, kwd_attrs, kwd_patterns } => {
                            // MatchClass: check isinstance(subject, cls) then check attributes.
                            // isinstance(subject, cls) consumes its `subject` argument off the
                            // stack (CALL pops callable + args) — without this DUP_TOP first,
                            // the subject that the rest of this case (sub-pattern DUP_TOPs, the
                            // final "pop subject" after the case body) assumes is still there
                            // is simply gone, corrupting the stack (underflow) for any `case
                            // ClassName():` pattern. The Or-pattern MatchClass arm above
                            // already does this DUP_TOP; this arm had been missing it.
                            self.emit(Opcode::DUP_TOP, 0);
                            let isinstance_idx = self.get_name_index("isinstance") as u32;
                            self.emit(Opcode::LOAD_GLOBAL, isinstance_idx);
                            self.emit(Opcode::SWAP, 1); // subject on top
                            self.compile_expr(cls)?;
                            self.emit(Opcode::CALL, 2); // isinstance(subject, cls)
                            self.emit_jump(Opcode::POP_JUMP_IF_FALSE, next_case);

                            // Check positional patterns (access by attribute or position)
                            for pat in patterns {
                                let _pat_next = self.new_label();
                                self.emit(Opcode::DUP_TOP, 0); // dup subject
                                match pat {
                                    Pattern::MatchValue(val) => {
                                        self.compile_expr(val)?;
                                        self.emit(Opcode::COMPARE_OP, 2); // ==
                                        self.emit_jump(Opcode::POP_JUMP_IF_FALSE, next_case);
                                    }
                                    Pattern::MatchAs { name: Some(n), .. } => {
                                        let idx = self.add_varname(n) as u32;
                                        self.emit(Opcode::STORE_FAST, idx);
                                    }
                                    Pattern::MatchAs { name: None, .. } => {
                                        self.emit(Opcode::POP_TOP, 0);
                                    }
                                    Pattern::MatchSingleton(s) => {
                                        let const_idx = self.get_const_index(match s.as_str() {
                                            "None" => ConstValue::None,
                                            "True" => ConstValue::Bool(true),
                                            "False" => ConstValue::Bool(false),
                                            _ => ConstValue::String(s.clone()),
                                        }) as u32;
                                        self.emit(Opcode::LOAD_CONST, const_idx);
                                        self.emit(Opcode::COMPARE_OP, 8); // IS
                                        self.emit_jump(Opcode::POP_JUMP_IF_FALSE, next_case);
                                    }
                                    _ => {}
                                }
                            }

                            // Check keyword patterns
                            for (kwd_attr, kwd_pat) in kwd_attrs.iter().zip(kwd_patterns.iter()) {
                                self.emit(Opcode::DUP_TOP, 0); // dup subject
                                let attr_idx = self.get_name_index(kwd_attr) as u32;
                                self.emit(Opcode::LOAD_ATTR, attr_idx);
                                match kwd_pat {
                                    Pattern::MatchValue(val) => {
                                        self.compile_expr(val)?;
                                        self.emit(Opcode::COMPARE_OP, 2); // ==
                                        self.emit_jump(Opcode::POP_JUMP_IF_FALSE, next_case);
                                    }
                                    Pattern::MatchAs { name: Some(n), .. } => {
                                        let idx = self.add_varname(n) as u32;
                                        self.emit(Opcode::STORE_FAST, idx);
                                    }
                                    Pattern::MatchAs { name: None, .. } => {
                                        self.emit(Opcode::POP_TOP, 0);
                                    }
                                    Pattern::MatchSingleton(s) => {
                                        let const_idx = self.get_const_index(match s.as_str() {
                                            "None" => ConstValue::None,
                                            "True" => ConstValue::Bool(true),
                                            "False" => ConstValue::Bool(false),
                                            _ => ConstValue::String(s.clone()),
                                        }) as u32;
                                        self.emit(Opcode::LOAD_CONST, const_idx);
                                        self.emit(Opcode::COMPARE_OP, 8); // IS
                                        self.emit_jump(Opcode::POP_JUMP_IF_FALSE, next_case);
                                    }
                                    _ => {}
                                }
                            }

                            if let Some(guard) = &case.guard {
                                self.compile_expr(guard)?;
                                self.emit_jump(Opcode::POP_JUMP_IF_FALSE, next_case);
                            }
                        }
                    }
                    self.emit(Opcode::POP_TOP, 0); // pop subject
                    self.compile_stmts(&case.body)?;
                    self.emit_jump(Opcode::JUMP, end_label);
                    self.fix_label(next_case);
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
                if self.scope == ScopeType::Module
                    || self.scope == ScopeType::ClassBody
                    || self.global_names.contains(name)
                {
                    let idx = self.get_name_index(name) as u32;
                    self.emit(Opcode::STORE_NAME, idx);
                } else if self.scope == ScopeType::Function && self.code.cellvars.contains(name) {
                    // Cell variable: use STORE_DEREF
                    let idx = self.code.cellvars.iter().position(|n| n == name).unwrap() as u32;
                    self.emit(Opcode::STORE_DEREF, idx);
                } else if self.scope == ScopeType::Function && self.code.freevars.contains(name) {
                    // Free variable (nonlocal): use STORE_DEREF
                    let fv_idx = self.code.freevars.iter().position(|n| n == name).unwrap();
                    let idx = (self.code.cellvars.len() + fv_idx) as u32;
                    self.emit(Opcode::STORE_DEREF, idx);
                } else {
                    let idx = self.add_varname(name) as u32;
                    self.emit(Opcode::STORE_FAST, idx);
                }
            }
            Expr::Attribute { value, attr } => {
                self.compile_expr(value)?;
                // Stack is [..., val, obj] — swap to [..., obj, val] for STORE_ATTR
                self.emit(Opcode::SWAP, 1);
                let idx = self.get_name_index(attr) as u32;
                self.emit(Opcode::STORE_ATTR, idx);
            }
            Expr::Subscript { value, slice } => {
                // Entered with the value-to-assign already on the stack
                // (chained assignment's COPY, or a tuple/list-unpack
                // target's per-element value) — stack is [value, obj, slice]
                // after pushing obj/slice below, but STORE_SUBSCR needs
                // [obj, slice, value] (matching the single-target
                // `Stmt::Assign` case above, which pushes obj/slice BEFORE
                // the value for exactly this reason). Reorder with two
                // SWAPs instead of restructuring the value-already-pushed
                // calling convention every caller of compile_assign_target
                // relies on.
                self.compile_expr(value)?;
                self.compile_expr(slice)?;
                self.emit(Opcode::SWAP, 1);
                self.emit(Opcode::SWAP, 2);
                self.emit(Opcode::STORE_SUBSCR, 0);
            }
            Expr::Starred(inner) => {
                // Starred target: unwrap and compile inner target
                self.compile_assign_target(inner)?;
            }
            Expr::List(elts) | Expr::Tuple(elts) => {
                // Check if any element is a Starred target — use UNPACK_EX if so
                let star_pos = elts.iter().position(|e| matches!(e, Expr::Starred(_)));
                if let Some(pos) = star_pos {
                    let before = pos;
                    let after = elts.len() - pos - 1;
                    let arg = ((before as u32) << 8) | (after as u32);
                    self.emit(Opcode::UNPACK_EX, arg);
                    for elt in elts {
                        self.compile_assign_target(elt)?;
                    }
                } else {
                    let count = elts.len();
                    self.emit(Opcode::UNPACK_SEQUENCE, count as u32);
                    for elt in elts {
                        self.compile_assign_target(elt)?;
                    }
                }
            }
            _ => return Err(format!("Cannot assign to {:?}", target)),
        }
        Ok(())
    }

    fn compile_function(
        &mut self,
        name: String,
        args: &[Arg],
        body: &[Stmt],
        is_async: bool,
    ) -> Result<(), String> {
        if std::env::var("RPY_DEBUG_COMPILE").is_ok() {
            eprintln!("compile_function: name={} is_async={}", name, is_async);
        }
        // Extract docstring from first statement if present
        let docstring = body.first().and_then(|s| {
            if let Stmt::Expr(expr) = Self::unwrap_located(s) {
                if let Expr::Constant(Constant::String(doc)) = expr.as_ref() {
                    Some(doc.clone())
                } else {
                    None
                }
            } else {
                None
            }
        });

        let body = if docstring.is_some() {
            
            &body[1..]
        } else {
            body
        };
        

        // Save outer code BEFORE enter_scope (which takes cellvars/freevars from self.code)
        let mut new_code = CodeObject::new(name.clone());
        new_code.filename = self.code.filename.clone();
        let old_code = std::mem::replace(&mut self.code, new_code);
        let old_labels = std::mem::replace(&mut self.labels, Vec::new());
        let old_label_stack = std::mem::replace(&mut self.label_stack, Vec::new());
        let old_loop_stack = std::mem::replace(&mut self.loop_stack, Vec::new());
        let old_with_stack = std::mem::replace(&mut self.pending_cleanup, Vec::new());
        let old_current_line = self.current_line;
        self.current_line = 1;
        let old_annotations_initialized = self.annotations_initialized;
        self.annotations_initialized = false;

        self.enter_scope(ScopeType::Function);
        self.varnames_stack.push(Self::enclosing_snapshot(&old_code));

        // Pre-analyze the function to determine cell vars and free vars.
        // Use the nearest REAL enclosing function's varnames (skipping over
        // any intervening class-body scopes) so module globals aren't
        // treated as free vars, while methods nested inside a class body
        // can still see past it to the function that encloses the class.
        let enclosing_varnames = self.compute_enclosing_names();
        let (cell_vars, free_vars) =
            Self::analyze_function(args, body, &self.global_names, &self.nonlocal_names, Some(&enclosing_varnames));
        self.code.cellvars = Box::new(cell_vars);
        self.code.freevars = Box::new(free_vars);

        // PEP 3135: Add __class__ as cell var for methods inside a class body
        if self.scope == ScopeType::Function {
            if let Some(outer) = self.scope_stack.last() {
                if outer.scope == ScopeType::ClassBody {
                    if !self.code.cellvars.contains(&"__class__".to_string()) {
                        self.code.cellvars.push("__class__".to_string());
                        if cfg!(feature = "profile") {  }
                    }
                }
            }
        }

        // Separate regular (positional-or-keyword) args from vararg/kwarg/
        // keyword-only ones. Keyword-only params (arg.is_kwonly — set by the
        // parser for anything after `*args` or a bare `*,` separator) must
        // NOT be folded into num_positional/defaults_count: they don't count
        // toward arg_count, and their defaults aren't positional defaults
        // (kwonly defaults can appear in any order — `def f(*, a=1, b)` — so
        // each slot's presence is tracked individually in
        // kwonly_defaults_mask rather than via a simple trailing count).
        let mut num_positional = 0;
        let mut defaults_count = 0;
        let mut kwonly_count = 0;
        let mut kwonly_defaults_mask = Vec::new();
        let mut arg_count_finalized = false;
        for arg in args {
            if arg.is_vararg {
                self.code.vararg_name = Some(Box::new(arg.arg.clone()));
                if !arg_count_finalized {
                    self.code.arg_count = num_positional;
                    arg_count_finalized = true;
                }
                continue;
            }
            if arg.is_kwarg {
                self.code.kwarg_name = Some(Box::new(arg.arg.clone()));
                continue;
            }
            if arg.is_kwonly {
                if !arg_count_finalized {
                    self.code.arg_count = num_positional;
                    arg_count_finalized = true;
                }
                kwonly_count += 1;
                kwonly_defaults_mask.push(arg.default.is_some());
                continue;
            }
            if arg.default.is_some() {
                defaults_count += 1;
            }
            num_positional += 1;
        }
        if !arg_count_finalized {
            self.code.arg_count = num_positional;
        }
        // Defaults are at the end of positional args, count them
        self.code.num_defaults = defaults_count;
        self.code.kwonlyarg_count = kwonly_count;
        self.code.kwonly_defaults_mask = Box::new(kwonly_defaults_mask);

        // Add all args to varnames (including vararg/kwarg at the end)
        for arg in args {
            self.add_varname(&arg.arg);
        }

        // Add cell vars to varnames too (so they get fast_locals slots)
        for cell_var in self.code.cellvars.clone().into_iter() {
            if self.get_var_index(&cell_var).is_none() {
                self.add_varname(&cell_var);
            }
        }

        // Emit MAKE_CELL for each cell var at function start
        for cell_var in self.code.cellvars.clone().iter() {
            if let Some(idx) = self.get_var_index(cell_var) {
                self.emit(Opcode::MAKE_CELL, idx as u32);
            }
        }

        // Check if function contains yield or is async (generator/coroutine)
        let has_yield = contains_yield_in_stmts(body) || is_async;
        if std::env::var("RPY_DEBUG_COMPILE").is_ok() {
            eprintln!("  -> {} has_yield={}", name, has_yield);
        }
        if has_yield {
            self.emit(Opcode::RETURN_GENERATOR, 0);
        }
        // Set CO_COROUTINE flag (0x100) for async functions
        if is_async {
            self.code.flags |= 0x100;
        }

        
        self.compile_stmts(body)?;
        

        // Implicit return None
        let has_return = body.iter().any(|s| matches!(s, Stmt::Return(_)));
        if !has_return {
            let const_none = self.get_const_index(ConstValue::None) as u32;
            self.emit(Opcode::LOAD_CONST, const_none);
            self.emit(Opcode::RETURN_VALUE, 0);
        }

        // Remember inner function's free vars for closure building
        let inner_free_vars = self.code.freevars.clone();
        let inner_cell_vars = self.code.cellvars.clone();
        if std::env::var("RPY_DEBUG_COMPILE").is_ok() && std::env::var("RPY_DEBUG_COMPILE_NAME").map(|n| n == name).unwrap_or(false) {
            eprintln!("  == {} instructions (cellvars={:?} freevars={:?} varnames={:?}) ==", name, inner_cell_vars, inner_free_vars, self.code.varnames);
            for (i, instr) in self.code.instructions.iter().enumerate() {
                eprintln!("    [{}] {:?} arg={}", i, instr.op, instr.arg);
            }
        }

        self.code.nlocals = self.code.varnames.len();
        self.code.name = name.clone();
        self.code.first_lineno = 1;

        self.code.cellvars = inner_cell_vars;
        self.code.freevars = inner_free_vars.clone();

        let func_code = std::mem::replace(&mut self.code, old_code);
        self.labels = old_labels;
        self.label_stack = old_label_stack;
        self.loop_stack = old_loop_stack;
        self.pending_cleanup = old_with_stack;
        self.current_line = old_current_line;
        self.annotations_initialized = old_annotations_initialized;
        self.varnames_stack.pop();
        // Leave the function's scope now, BEFORE compiling default-value
        // expressions below — defaults are evaluated once in the enclosing
        // scope at def-time (matching Python semantics), and if a default is
        // itself a lambda (`def f(x=lambda: 1)`), compiling it recursively
        // calls compile_function again. Leaving this until the end (after
        // defaults) left self.scope/scope_stack one level "too deep" for
        // that window relative to varnames_stack (already popped above),
        // corrupting compute_enclosing_names for the nested lambda.
        self.leave_scope();

        // Emit LOAD_CLOSURE for each free var of the inner function
        let mut nfree = 0usize;
        for fv_name in inner_free_vars.iter() {
            let found = self.code.cellvars.iter().any(|n| n == fv_name)
                || self.code.freevars.iter().any(|n| n == fv_name)
                || self.get_var_index(fv_name).is_some();
            if found {
                if self.get_var_index(fv_name).is_some() && !self.code.cellvars.contains(fv_name) {
                    self.code.cellvars.push(fv_name.clone());
                    if self.get_var_index(fv_name).is_none() {
                        self.add_varname(fv_name);
                    }
                }
                // A name relayed from further out that we (as the
                // intervening scope) also expose as one of our *own*
                // cellvars purely so a nested function can see it (see
                // `analyze_function`'s cell_vars doc comment) is present in
                // BOTH lists here — but only the freevar slot actually
                // holds the real, already-populated cell (received via our
                // own closure); the cellvar slot is a fresh, empty one
                // `MAKE_CELL` created at our own scope's start, never
                // written to, since we only ever *read* the relayed value
                // via the freevar path ourselves. Check freevars first so
                // relaying threads the same real cell through, instead of
                // handing a nested function an uninitialized one.
                if let Some(idx) = self.code.freevars.iter().position(|n| n == fv_name) {
                    let idx = self.code.cellvars.len() + idx;
                    self.emit(Opcode::LOAD_CLOSURE, idx as u32);
                } else if let Some(idx) = self.code.cellvars.iter().position(|n| n == fv_name) {
                    self.emit(Opcode::LOAD_CLOSURE, idx as u32);
                }
                nfree += 1;
            }
        }
        if nfree > 0 {
            self.emit(Opcode::BUILD_TUPLE, nfree as u32);
        }

        let kwonly_defaults_count = args.iter().filter(|a| a.is_kwonly && a.default.is_some()).count();
        let mut make_func_arg = defaults_count as u32;
        if nfree > 0 {
            make_func_arg |= 1 << 8;
        }
        // Bits 9-16: count of keyword-only defaults (see MAKE_FUNCTION —
        // popped after the positional defaults, appended to
        // PyObject::Function.defaults right after them).
        make_func_arg |= (kwonly_defaults_count as u32) << 9;
        let code_const_idx = self.get_const_index(ConstValue::Code(Box::new(func_code))) as u32;
        self.emit(Opcode::LOAD_CONST, code_const_idx);

        // Push defaults onto stack (in normal order, they'll be reversed in
        // MAKE_FUNCTION) — positional first, then keyword-only (only those
        // that actually have one; kwonly defaults may be sparse, e.g.
        // `def f(*, a=1, b)`, unlike positional defaults which are always a
        // trailing run).
        if defaults_count > 0 {
            for arg in args
                .iter()
                .filter(|a| !a.is_vararg && !a.is_kwarg && !a.is_kwonly && a.default.is_some())
            {
                if let Some(default) = &arg.default {
                    self.compile_expr(default)?;
                }
            }
        }
        if kwonly_defaults_count > 0 {
            for arg in args.iter().filter(|a| a.is_kwonly && a.default.is_some()) {
                if let Some(default) = &arg.default {
                    self.compile_expr(default)?;
                }
            }
        }

        self.emit(Opcode::MAKE_FUNCTION, make_func_arg);

        // Set __doc__ if there was a docstring
        if let Some(doc) = docstring {
            self.emit(Opcode::DUP_TOP, 0);
            let doc_idx = self.get_const_index(ConstValue::String(doc)) as u32;
            self.emit(Opcode::LOAD_CONST, doc_idx);
            let doc_attr_idx = self.get_name_index("__doc__") as u32;
            self.emit(Opcode::STORE_ATTR, doc_attr_idx);
        }

        Ok(())
    }

    fn compile_class_body(&mut self, name: String, body: &[Stmt]) -> Result<(), String> {
        

        // Skip docstring if first statement is a string literal
        let body = if let Some(Stmt::Expr(expr)) = body.first().map(Self::unwrap_located) {
            if matches!(expr.as_ref(), Expr::Constant(Constant::String(_))) {
                &body[1..]
            } else {
                body
            }
        } else {
            body
        };

        self.enter_scope(ScopeType::ClassBody);
        self.class_name_stack.push(name.clone());

        let mut new_class_code = CodeObject::new(name.clone());
        new_class_code.filename = self.code.filename.clone();
        let old_code = std::mem::replace(&mut self.code, new_class_code);
        self.varnames_stack.push(Self::enclosing_snapshot(&old_code));

        let old_labels = std::mem::replace(&mut self.labels, Vec::new());
        let old_label_stack = std::mem::replace(&mut self.label_stack, Vec::new());
        let old_loop_stack = std::mem::replace(&mut self.loop_stack, Vec::new());
        let old_with_stack = std::mem::replace(&mut self.pending_cleanup, Vec::new());
        let old_current_line = self.current_line;
        self.current_line = 1;
        let old_annotations_initialized = self.annotations_initialized;
        self.annotations_initialized = false;

        self.code.arg_count = 0;

        // Class bodies are skipped when Python resolves enclosing scope, but
        // methods defined here can still close over the enclosing function's
        // locals — so this class body's code object needs those relayed
        // through as free variables, exactly like a nested function would.
        let enclosing_varnames = self.compute_enclosing_names();
        let (_ignored_cellvars, free_vars) = Self::analyze_function(
            &[],
            body,
            &self.global_names,
            &self.nonlocal_names,
            Some(&enclosing_varnames),
        );
        self.code.freevars = Box::new(free_vars);

        // Real Python implicitly seeds __module__ = <enclosing module's
        // __name__> and __qualname__ = <class name> as the first two
        // statements of every class body, before any user code runs. We
        // weren't seeding either — real metaclasses very commonly assume
        // __module__ is always present in the namespace dict they receive
        // (e.g. Django's `ModelBase.__new__`: `attrs.pop("__module__")`,
        // unconditional, no default), so its absence surfaced as a raw
        // KeyError deep inside ordinary class creation.
        {
            let module_name_idx = self.get_name_index("__name__") as u32;
            self.emit(Opcode::LOAD_NAME, module_name_idx);
            let module_attr_idx = self.get_name_index("__module__") as u32;
            self.emit(Opcode::STORE_NAME, module_attr_idx);

            let qualname_const_idx = self.get_const_index(ConstValue::String(name.clone())) as u32;
            self.emit(Opcode::LOAD_CONST, qualname_const_idx);
            let qualname_attr_idx = self.get_name_index("__qualname__") as u32;
            self.emit(Opcode::STORE_NAME, qualname_attr_idx);
        }

        self.compile_stmts(body)?;

        let const_none = self.get_const_index(ConstValue::None) as u32;
        self.emit(Opcode::LOAD_CONST, const_none);
        self.emit(Opcode::RETURN_VALUE, 0);

        self.code.nlocals = self.code.varnames.len();
        self.code.name = name.clone();
        self.code.first_lineno = 1;

        let inner_free_vars = self.code.freevars.clone();

        let func_code = std::mem::replace(&mut self.code, old_code);
        self.labels = old_labels;
        self.label_stack = old_label_stack;
        self.loop_stack = old_loop_stack;
        self.pending_cleanup = old_with_stack;
        self.current_line = old_current_line;
        self.annotations_initialized = old_annotations_initialized;
        self.varnames_stack.pop();

        // Relay any free variables this class body's methods need, using the
        // same mechanism as an ordinary nested function.
        let mut nfree = 0usize;
        for fv_name in inner_free_vars.iter() {
            let found = self.code.cellvars.iter().any(|n| n == fv_name)
                || self.code.freevars.iter().any(|n| n == fv_name)
                || self.get_var_index(fv_name).is_some();
            if found {
                if self.get_var_index(fv_name).is_some() && !self.code.cellvars.contains(fv_name) {
                    self.code.cellvars.push(fv_name.clone());
                    if self.get_var_index(fv_name).is_none() {
                        self.add_varname(fv_name);
                    }
                }
                // A name relayed from further out that we (as the
                // intervening scope) also expose as one of our *own*
                // cellvars purely so a nested function can see it (see
                // `analyze_function`'s cell_vars doc comment) is present in
                // BOTH lists here — but only the freevar slot actually
                // holds the real, already-populated cell (received via our
                // own closure); the cellvar slot is a fresh, empty one
                // `MAKE_CELL` created at our own scope's start, never
                // written to, since we only ever *read* the relayed value
                // via the freevar path ourselves. Check freevars first so
                // relaying threads the same real cell through, instead of
                // handing a nested function an uninitialized one.
                if let Some(idx) = self.code.freevars.iter().position(|n| n == fv_name) {
                    let idx = self.code.cellvars.len() + idx;
                    self.emit(Opcode::LOAD_CLOSURE, idx as u32);
                } else if let Some(idx) = self.code.cellvars.iter().position(|n| n == fv_name) {
                    self.emit(Opcode::LOAD_CLOSURE, idx as u32);
                }
                nfree += 1;
            }
        }
        if nfree > 0 {
            self.emit(Opcode::BUILD_TUPLE, nfree as u32);
        }

        let code_const_idx = self.get_const_index(ConstValue::Code(Box::new(func_code))) as u32;
        self.emit(Opcode::LOAD_CONST, code_const_idx);
        let make_func_arg: u32 = if nfree > 0 { 1 << 8 } else { 0 };
        self.emit(Opcode::MAKE_FUNCTION, make_func_arg);

        self.leave_scope();
        self.class_name_stack.pop();
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
                    Constant::Bytes(b) => ConstValue::Bytes(b.clone()),
                    Constant::Complex { real, imag } => ConstValue::Complex { real: real.clone(), imag: imag.clone() },
                };
                let idx = self.get_const_index(const_value) as u32;
                self.emit(Opcode::LOAD_CONST, idx);
            }
            Expr::Name(name) => {
                if std::env::var("RPY_DEBUG_COMPILE_NAME_RESOLVE").ok().as_deref() == Some(name.as_str()) {
                    eprintln!("NAME_RESOLVE: name={} scope={:?} in_global_names={} freevars={:?} cellvars={:?}", name, self.scope, self.global_names.contains(name), self.code.freevars, self.code.cellvars);
                }
                if self.scope == ScopeType::Module
                    || self.scope == ScopeType::ClassBody
                    || self.global_names.contains(name)
                {
                    let name_idx = self.get_name_index(name) as u32;
                    
                    self.emit(Opcode::LOAD_NAME, name_idx);
                } else if self.scope == ScopeType::Function && self.code.freevars.contains(name) {
                    let fv_idx = self.code.freevars.iter().position(|n| n == name).unwrap();
                    let idx = self.code.cellvars.len() + fv_idx;
                    
                    self.emit(Opcode::LOAD_DEREF, idx as u32);
                } else if self.scope == ScopeType::Function && self.code.cellvars.contains(name) {
                    let idx = self.code.cellvars.iter().position(|n| n == name).unwrap() as u32;
                    
                    self.emit(Opcode::LOAD_DEREF, idx);
                } else if self.scope == ScopeType::Function && self.get_var_index(name).is_some() {
                    let idx = self.get_var_index(name).unwrap() as u32;
                    
                    self.emit(Opcode::LOAD_FAST, idx);
                } else if self.scope == ScopeType::Function {
                    let name_idx = self.get_name_index(name) as u32;
                    
                    self.emit(Opcode::LOAD_GLOBAL, name_idx);
                } else {
                    let name_idx = self.get_name_index(name) as u32;
                    
                    self.emit(Opcode::LOAD_NAME, name_idx);
                }
            }
            Expr::BinOp { left, op, right } => {
                // Constant folding: compute 3+4 etc. at compile time
                if let (Expr::Constant(Constant::Int(a)), Expr::Constant(Constant::Int(b))) = (&**left, &**right) {
                    let result = match op {
                        Operator::Add => a.parse::<i64>().ok().zip(b.parse::<i64>().ok()).map(|(x,y)| x + y),
                        Operator::Sub => a.parse::<i64>().ok().zip(b.parse::<i64>().ok()).map(|(x,y)| x - y),
                        Operator::Mult => a.parse::<i64>().ok().zip(b.parse::<i64>().ok()).map(|(x,y)| x * y),
                        _ => None,
                    };
                    if let Some(val) = result {
                        let idx = self.get_const_index(ConstValue::Int(val.to_string())) as u32;
                        self.emit(Opcode::LOAD_CONST, idx);
                        return Ok(());
                    }
                }
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
                for (i, val) in values.iter().enumerate() {
                    self.compile_expr(val)?;
                    if i < values.len() - 1 {
                        self.emit(Opcode::DUP_TOP, 0);
                        let jump_op = match op {
                            BoolOp::And => Opcode::POP_JUMP_IF_FALSE,
                            BoolOp::Or => Opcode::POP_JUMP_IF_TRUE,
                        };
                        self.emit_jump(jump_op, end_label);
                        self.emit(Opcode::POP_TOP, 0);
                    }
                }
                self.fix_label(end_label);
            }
            Expr::Compare {
                left,
                ops,
                comparators,
            } => {
                let chained_end = self.new_label();
                for (i, (op, right)) in ops.iter().zip(comparators.iter()).enumerate() {
                    if i > 0 {
                        // Chained comparison: re-compile previous comparand as new left
                        self.compile_expr(&comparators[i - 1])?;
                    } else {
                        self.compile_expr(left)?;
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
                    if i < ops.len() - 1 {
                        self.emit(Opcode::DUP_TOP, 0);
                        self.emit_jump(Opcode::POP_JUMP_IF_FALSE, chained_end);
                        self.emit(Opcode::POP_TOP, 0);
                    }
                }
                self.fix_label(chained_end);
            }
            Expr::Call {
                func,
                args,
                keywords,
            } => {
                // PEP 3135: super() without args in methods -> super(__class__, self)
                let mut extra_pos = 0usize;
                let is_bare_super = if let Expr::Name(n) = func.as_ref() {
                    n == "super" && args.is_empty() && keywords.is_empty()
                } else {
                    false
                };
                if is_bare_super {
                    // PEP 3135: super() without args resolves against
                    // __class__ (the class this method is textually defined
                    // in) and self — NOT type(self) (the instance's runtime
                    // type), which this used to inject instead. That broke
                    // any 3+-level hierarchy where a method is called on a
                    // subclass instance that doesn't override it: e.g.
                    // Widget -> Input -> HiddenInput, HiddenInput() calling
                    // inherited Input.__init__'s `super().__init__()`. With
                    // cls=type(self)=HiddenInput, "the class after
                    // HiddenInput in HiddenInput's own MRO" is Input —
                    // Input.__init__ again — infinite self-recursion (stack
                    // overflow) instead of reaching Widget.__init__. Since
                    // the class object doesn't exist yet at compile time
                    // (methods compile before the class is built), look it
                    // up by name instead — correct as long as the class is
                    // still bound to its defining name by the time any
                    // instance method actually runs, true for essentially
                    // all real code (`class Foo: ...` always binds "Foo").
                    if self.scope == ScopeType::Function && !self.code.varnames.is_empty() {
                        if let Some(class_name) = self.class_name_stack.last().cloned() {
                            self.compile_expr(func)?;
                            let class_name_idx = self.get_name_index(&class_name) as u32;
                            self.emit(Opcode::LOAD_GLOBAL, class_name_idx);
                            self.emit(Opcode::LOAD_FAST, 0);
                            extra_pos = 2;
                        } else {
                            // Bare super() outside a class body is invalid
                            // Python anyway; keep the old best-effort behavior.
                            self.compile_expr(func)?;
                            let type_name_idx = self.get_name_index("type") as u32;
                            self.emit(Opcode::LOAD_GLOBAL, type_name_idx);
                            self.emit(Opcode::LOAD_FAST, 0);
                            self.emit(Opcode::CALL, 1);
                            self.emit(Opcode::LOAD_FAST, 0);
                            extra_pos = 2;
                        }
                    } else {
                        self.compile_expr(func)?;
                    }
                } else {
                    self.compile_expr(func)?;
                }
                // f(*args) / f(x, *args) / f(**kwargs): a starred positional
                // or bare-** keyword must be unpacked at call time, not
                // passed through as a single tuple/dict value — the plain
                // CALL opcode below has no way to express "this many of my
                // arguments came from unpacking", so use CALL_FUNCTION_EX
                // instead, matching CPython's own split between the two.
                let has_star_args = args.iter().any(|a| matches!(a, Expr::Starred(_)));
                let has_star_kwargs = keywords.iter().any(|kw| kw.arg.is_none());
                if has_star_args || has_star_kwargs {
                    self.emit(Opcode::BUILD_LIST, 0);
                    for arg in args {
                        if let Expr::Starred(inner) = arg {
                            self.compile_expr(inner)?;
                            self.emit(Opcode::LIST_EXTEND, 0);
                        } else {
                            self.compile_expr(arg)?;
                            self.emit(Opcode::LIST_APPEND, 0);
                        }
                    }
                    self.emit(Opcode::LIST_TO_TUPLE, 0);
                    self.emit(Opcode::BUILD_MAP, 0);
                    for kw in keywords {
                        if let Some(name) = &kw.arg {
                            let name_idx = self.get_const_index(ConstValue::String(name.clone())) as u32;
                            self.emit(Opcode::LOAD_CONST, name_idx);
                            self.compile_expr(&kw.value)?;
                            self.emit(Opcode::MAP_ADD, 0);
                        } else {
                            self.compile_expr(&kw.value)?;
                            self.emit(Opcode::DICT_MERGE, 0);
                        }
                    }
                    self.emit(Opcode::CALL_FUNCTION_EX, 0);
                    return Ok(());
                }

                let npos = args.len() + extra_pos;
                let nkw = keywords.len();

                for arg in args {
                    self.compile_expr(arg)?;
                }
                for kw in keywords {
                    if let Some(name) = &kw.arg {
                        let name_idx =
                            self.get_const_index(ConstValue::String(name.clone())) as u32;
                        self.emit(Opcode::LOAD_CONST, name_idx);
                        self.compile_expr(&kw.value)?;
                    } else {
                        // **kwargs
                        self.compile_expr(&kw.value)?;
                    }
                }
                let call_arg = npos | (nkw << 8);
                self.emit(Opcode::CALL, call_arg as u32);
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
                self.compile_function(
                    "<lambda>".to_string(),
                    args,
                    &[Stmt::Return(Some(body.clone()))],
                    false,
                )?;
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
                if elts.iter().any(|e| matches!(e, Expr::Starred(_))) {
                    // Star unpacking present — use incremental building with LIST_APPEND/LIST_EXTEND
                    self.emit(Opcode::BUILD_LIST, 0);
                    for elt in elts {
                        if let Expr::Starred(inner) = elt {
                            self.compile_expr(inner)?;
                            self.emit(Opcode::LIST_EXTEND, 0);
                        } else {
                            self.compile_expr(elt)?;
                            self.emit(Opcode::LIST_APPEND, 0);
                        }
                    }
                } else {
                    for elt in elts {
                        self.compile_expr(elt)?;
                    }
                    self.emit(Opcode::BUILD_LIST, elts.len() as u32);
                }
            }
            Expr::Tuple(elts) => {
                if elts.iter().any(|e| matches!(e, Expr::Starred(_))) {
                    // Star unpacking present — build incrementally as a list
                    // (LIST_APPEND/LIST_EXTEND handle Starred correctly),
                    // then convert to a tuple.
                    self.emit(Opcode::BUILD_LIST, 0);
                    for elt in elts {
                        if let Expr::Starred(inner) = elt {
                            self.compile_expr(inner)?;
                            self.emit(Opcode::LIST_EXTEND, 0);
                        } else {
                            self.compile_expr(elt)?;
                            self.emit(Opcode::LIST_APPEND, 0);
                        }
                    }
                    self.emit(Opcode::LIST_TO_TUPLE, 0);
                } else {
                    for elt in elts {
                        self.compile_expr(elt)?;
                    }
                    self.emit(Opcode::BUILD_TUPLE, elts.len() as u32);
                }
            }
            Expr::Dict { keys, values } => {
                self.emit(Opcode::BUILD_MAP, 0);
                let mut dup_count = 0u32;
                for (key, value) in keys.iter().zip(values.iter()) {
                    match key {
                        Some(k) => {
                            self.emit(Opcode::DUP_TOP, 0);
                            dup_count += 1;
                            self.compile_expr(k)?;
                            self.compile_expr(value)?;
                            self.emit(Opcode::MAP_ADD, 1);
                        }
                        None => {
                            // Dict unpacking: {**expr}
                            self.emit(Opcode::DUP_TOP, 0);
                            dup_count += 1;
                            self.compile_expr(value)?;
                            self.emit(Opcode::DICT_MERGE, 1);
                        }
                    }
                }
                // Pop all DUP_TOP copies except the original BUILD_MAP result
                for _ in 0..dup_count {
                    self.emit(Opcode::POP_TOP, 0);
                }
            }
            Expr::Set(elts) => {
                if elts.iter().any(|e| matches!(e, Expr::Starred(_))) {
                    // Star unpacking present (`{*a, *b}`) — build incrementally,
                    // mirroring the Tuple case above.
                    self.emit(Opcode::BUILD_SET, 0);
                    for elt in elts {
                        if let Expr::Starred(inner) = elt {
                            self.compile_expr(inner)?;
                            self.emit(Opcode::SET_UPDATE, 0);
                        } else {
                            self.compile_expr(elt)?;
                            self.emit(Opcode::SET_ADD, 0);
                        }
                    }
                } else {
                    for elt in elts {
                        self.compile_expr(elt)?;
                    }
                    self.emit(Opcode::BUILD_SET, elts.len() as u32);
                }
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
                if let Some(l) = lower {
                    self.compile_expr(l)?;
                } else {
                    self.emit(Opcode::LOAD_CONST, const_none);
                }
                if let Some(u) = upper {
                    self.compile_expr(u)?;
                } else {
                    self.emit(Opcode::LOAD_CONST, const_none);
                }
                if step.is_some() {
                    if let Some(s) = step {
                        self.compile_expr(s)?;
                    }
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
                let mut count = 0usize;
                for part in parts {
                    match part {
                        FStringPart::String(s) => {
                            self.compile_expr(&Expr::Constant(Constant::String(s.clone())))?;
                            count += 1;
                        }
                        FStringPart::Expr { expr, conversion, format_spec } => {
                            self.compile_expr(&expr)?;
                            if *conversion != 0 {
                                self.emit(Opcode::CONVERT_VALUE, *conversion as u32);
                            }
                            if let Some(spec) = format_spec {
                                self.compile_expr(&spec)?;
                                self.emit(Opcode::FORMAT_WITH_SPEC, 0);
                            } else if *conversion == 0 {
                                self.emit(Opcode::FORMAT_SIMPLE, 0);
                            }
                            count += 1;
                        }
                    }
                }
                if count > 1 {
                    self.emit(Opcode::BUILD_STRING, count as u32);
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
                self.emit(Opcode::DUP_TOP, 0);
                self.compile_assign_target(target)?;
            }
            Expr::DictComp {
                key,
                value,
                generators,
            } => {
                self.compile_dict_comprehension(key, value, generators)?;
            }
            Expr::YieldFrom(expr) => {
                // Simple yield from: iterate and yield each value
                self.compile_expr(expr)?;
                self.emit(Opcode::GET_ITER, 0);
                let end_label = self.new_label();
                let loop_label = self.new_label();
                self.mark_label(loop_label);
                self.emit_jump(Opcode::FOR_ITER, end_label);
                self.emit(Opcode::YIELD_VALUE, 0);
                self.emit(Opcode::POP_TOP, 0);
                self.emit_backward_jump(loop_label);
                self.fix_label(end_label);
                self.emit(Opcode::POP_ITER, 0);
                let const_none = self.get_const_index(ConstValue::None) as u32;
                self.emit(Opcode::LOAD_CONST, const_none);
            }
            Expr::Await(expr) => {
                self.compile_expr(expr)?;
                self.emit(Opcode::GET_AWAITABLE, 0);
                let const_none = self.get_const_index(ConstValue::None) as u32;
                self.emit(Opcode::LOAD_CONST, const_none);
                // Emit the SEND/YIELD loop (matching CPython's `await` compilation):
                //   >> loop:
                //     SEND cleanup_target   (jump to cleanup_target on StopIteration)
                //     YIELD_VALUE           (yield the awaited value)
                //     JUMP_BACKWARD loop    (the resume value from send() is used
                //                           as the next SEND value)
                //   >> cleanup_target:
                //     END_SEND             (pop iterator, pop result, push result)
                let loop_label = self.new_label();
                let end_label = self.new_label();
                self.mark_label(loop_label);
                self.emit_jump(Opcode::SEND, end_label);
                self.emit(Opcode::YIELD_VALUE, 0);
                self.emit_backward_jump(loop_label);
                self.fix_label(end_label);
                self.emit(Opcode::END_SEND, 0);
            }
        }
        Ok(())
    }

    fn compile_comprehension(
        &mut self,
        elt: &Expr,
        generators: &[Comprehension],
        is_set: bool,
    ) -> Result<(), String> {
        if generators.is_empty() {
            return Err("Comprehension must have at least one generator".to_string());
        }

        if is_set {
            self.emit(Opcode::BUILD_SET, 0);
        } else {
            self.emit(Opcode::BUILD_LIST, 0);
        }

        let num_gen = generators.len();
        let mut start_labels = Vec::with_capacity(num_gen);
        let mut continue_labels = Vec::with_capacity(num_gen);
        let end_label = self.new_label();

        for (i, gen) in generators.iter().enumerate() {
            self.compile_expr(&gen.iter)?;
            self.emit(Opcode::GET_ITER, 0);
            let start_label = self.new_label();
            start_labels.push(start_label);
            self.mark_label(start_label);

            if i == 0 {
                // Outermost FOR_ITER — when exhausted, jump to end
                self.emit_jump(Opcode::FOR_ITER, end_label);
            } else {
                // Inner FOR_ITER — when exhausted, pop this iter and go back to parent
                let cont_label = self.new_label();
                continue_labels.push(cont_label);
                self.emit_jump(Opcode::FOR_ITER, cont_label);
            }

            self.compile_assign_target(&gen.target)?;

            // A failed `if` clause must skip straight to fetching this
            // generator's next item — NOT fall through to `elt`/APPEND
            // regardless, which is what this previously did: it jumped
            // over a single NOP placed immediately before `fix_label`, so
            // both the true and false branches landed on the exact same
            // next instruction and the condition had no effect at all
            // (`[x for x in seq if cond]` included every element,
            // condition ignored). POP_JUMP_IF_FALSE takes an *absolute*
            // instruction position (see its VM handler), and this
            // generator's own start_label was already `mark_label`ed
            // above, so its position is already known here — no
            // forward-label/fix_label bookkeeping needed, same as how
            // `emit_backward_jump` below reuses an already-marked position.
            let continue_pos = self.label_positions[*start_labels.last().unwrap()] as u32;
            for if_expr in &gen.ifs {
                self.compile_expr(if_expr)?;
                self.emit(Opcode::POP_JUMP_IF_FALSE, continue_pos);
            }
        }

        self.compile_expr(elt)?;
        let depth = generators.len() as u32;
        if is_set {
            self.emit(Opcode::SET_ADD, depth);
        } else {
            self.emit(Opcode::LIST_APPEND, depth);
        }

        self.emit_backward_jump(*start_labels.last().unwrap());

        for (j, label) in continue_labels.iter().enumerate().rev() {
            self.fix_label(*label);
            self.emit(Opcode::POP_ITER, 0);
            self.emit_backward_jump(start_labels[j]);
        }

        self.fix_label(end_label);
        self.emit(Opcode::POP_ITER, 0);

        Ok(())
    }

    fn compile_dict_comprehension(
        &mut self,
        key: &Expr,
        value: &Expr,
        generators: &[Comprehension],
    ) -> Result<(), String> {
        if generators.is_empty() {
            return Err("Comprehension must have at least one generator".to_string());
        }

        self.emit(Opcode::BUILD_MAP, 0);

        let num_gen = generators.len();
        let mut start_labels = Vec::with_capacity(num_gen);
        let mut continue_labels = Vec::with_capacity(num_gen);
        let end_label = self.new_label();

        for (i, gen) in generators.iter().enumerate() {
            self.compile_expr(&gen.iter)?;
            self.emit(Opcode::GET_ITER, 0);
            let start_label = self.new_label();
            start_labels.push(start_label);
            self.mark_label(start_label);

            if i == 0 {
                self.emit_jump(Opcode::FOR_ITER, end_label);
            } else {
                let cont_label = self.new_label();
                continue_labels.push(cont_label);
                self.emit_jump(Opcode::FOR_ITER, cont_label);
            }

            self.compile_assign_target(&gen.target)?;

            // See the matching fix in compile_comprehension for why this
            // can't use the forward-label/NOP pattern (it made the filter
            // condition a no-op — every item passed regardless).
            let continue_pos = self.label_positions[*start_labels.last().unwrap()] as u32;
            for if_expr in &gen.ifs {
                self.compile_expr(if_expr)?;
                self.emit(Opcode::POP_JUMP_IF_FALSE, continue_pos);
            }
        }

        self.compile_expr(key)?;
        self.compile_expr(value)?;
        self.emit(Opcode::MAP_ADD, generators.len() as u32);

        self.emit_backward_jump(*start_labels.last().unwrap());

        for (j, label) in continue_labels.iter().enumerate().rev() {
            self.fix_label(*label);
            self.emit(Opcode::POP_ITER, 0);
            self.emit_backward_jump(start_labels[j]);
        }

        self.fix_label(end_label);
        self.emit(Opcode::POP_ITER, 0);

        Ok(())
    }
}

fn contains_yield_in_stmts(stmts: &[Stmt]) -> bool {
    stmts.iter().any(|s| match Compiler::unwrap_located(s) {
        Stmt::Expr(expr)
        | Stmt::Return(Some(expr))
        | Stmt::Assign { value: expr, .. }
        | Stmt::AugAssign { value: expr, .. } => contains_yield_in_expr(expr),
        Stmt::If { test, body, orelse } => {
            contains_yield_in_expr(test)
                || contains_yield_in_stmts(body)
                || contains_yield_in_stmts(orelse)
        }
        Stmt::While { test, body, orelse } => {
            contains_yield_in_expr(test)
                || contains_yield_in_stmts(body)
                || contains_yield_in_stmts(orelse)
        }
        Stmt::For {
            iter, body, orelse, ..
        } => {
            contains_yield_in_expr(iter)
                || contains_yield_in_stmts(body)
                || contains_yield_in_stmts(orelse)
        }
        Stmt::With { items, body, .. } => {
            items
                .iter()
                .any(|i| contains_yield_in_expr(&i.context_expr))
                || contains_yield_in_stmts(body)
        }
        Stmt::Try {
            body,
            handlers,
            handlers_star,
            orelse,
            finalbody,
        } => {
                contains_yield_in_stmts(body)
                || handlers.iter().any(|h| contains_yield_in_stmts(&h.body))
                || handlers_star.iter().any(|h| contains_yield_in_stmts(&h.body))
                || contains_yield_in_stmts(orelse)
                || contains_yield_in_stmts(finalbody)
        }
        // A nested `def`/`async def`/`class` starts its own independent
        // scope — whether *it* contains `yield`/`await` has no bearing on
        // whether the *enclosing* function is a generator/coroutine. This
        // used to recurse into the nested body, so e.g. a plain nested
        // helper `def decorator(func): ... async def wrapper(...): return
        // await func(...) ... return wrapper` (real code:
        // `django.utils.deprecation.deprecate_posargs`, an ordinary
        // sync/async-dispatching decorator factory, no yield/await
        // anywhere in its own body) made every *enclosing* function
        // wrongly compiled as a generator too — calling it returned a bare
        // generator object instead of ever running its body, since nothing
        // actually executes until the generator is iterated. Confirmed
        // minimal repro: a function returning a nested function containing
        // only a conditionally-defined `async def` sibling came back as
        // `<generator object>` instead of the callable it should return.
        Stmt::FunctionDef { .. } | Stmt::ClassDef { .. } => false,
        _ => false,
    })
}

fn contains_yield_in_expr(expr: &Expr) -> bool {
    match expr {
        Expr::Yield(_) => true,
        Expr::YieldFrom(_) => true,
        Expr::Await(_) => true,
        Expr::BinOp { left, right, .. } => {
            contains_yield_in_expr(left) || contains_yield_in_expr(right)
        }
        Expr::BoolOp { values, .. } => values.iter().any(contains_yield_in_expr),
        Expr::Compare {
            left, comparators, ..
        } => contains_yield_in_expr(left) || comparators.iter().any(contains_yield_in_expr),
        Expr::UnaryOp { operand, .. } => contains_yield_in_expr(operand),
        Expr::IfExp { test, body, orelse } => {
            contains_yield_in_expr(test)
                || contains_yield_in_expr(body)
                || contains_yield_in_expr(orelse)
        }
        Expr::Lambda { body, .. } => contains_yield_in_expr(body),
        Expr::Call {
            func,
            args,
            keywords,
        } => {
            contains_yield_in_expr(func)
                || args.iter().any(contains_yield_in_expr)
                || keywords.iter().any(|k| contains_yield_in_expr(&k.value))
        }
        Expr::Attribute { value, .. } => contains_yield_in_expr(value),
        Expr::Subscript { value, slice } => {
            contains_yield_in_expr(value) || contains_yield_in_expr(slice)
        }
        Expr::List(elts) | Expr::Tuple(elts) => elts.iter().any(contains_yield_in_expr),
        Expr::Dict { keys, values } => {
            keys.iter()
                .any(|k| k.as_ref().map_or(false, |e| contains_yield_in_expr(e)))
                || values.iter().any(contains_yield_in_expr)
        }
        Expr::Starred(expr) => contains_yield_in_expr(expr),
        Expr::ListComp { elt, generators } | Expr::SetComp { elt, generators } => {
            contains_yield_in_expr(elt)
                || generators.iter().any(|g| {
                    contains_yield_in_expr(&g.iter)
                        || contains_yield_in_expr(&g.target)
                        || g.ifs.iter().any(|e| contains_yield_in_expr(e))
                })
        }
        Expr::DictComp {
            key,
            value,
            generators,
        } => {
            contains_yield_in_expr(key)
                || contains_yield_in_expr(value)
                || generators.iter().any(|g| {
                    contains_yield_in_expr(&g.iter)
                        || contains_yield_in_expr(&g.target)
                        || g.ifs.iter().any(|e| contains_yield_in_expr(e))
                })
        }
        Expr::GeneratorExp { elt, generators } => {
            contains_yield_in_expr(elt)
                || generators.iter().any(|g| {
                    contains_yield_in_expr(&g.iter)
                        || contains_yield_in_expr(&g.target)
                        || g.ifs.iter().any(|e| contains_yield_in_expr(e))
                })
        }
        // An f-string's embedded expressions can contain `await` (legal in
        // an async function: `f"{await foo()}"`) — see the matching fix in
        // `collect_names_expr` for why treating the whole f-string as
        // opaque is wrong in general.
        Expr::FString(parts) => parts.iter().any(|p| match p {
            FStringPart::Expr { expr, format_spec, .. } => {
                contains_yield_in_expr(expr)
                    || format_spec.as_ref().is_some_and(|fs| contains_yield_in_expr(fs))
            }
            FStringPart::String(_) => false,
        }),
        Expr::JoinedStr(exprs) => exprs.iter().any(contains_yield_in_expr),
        _ => false,
    }
}
