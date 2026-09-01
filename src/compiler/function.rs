use crate::ast::*;
use crate::bytecode::*;
use super::Compiler;
use super::scope::{PendingCleanup, ScopeType};
use super::utils::contains_yield_in_stmts;

impl Compiler {
    pub(crate) fn compile_function(
        &mut self,
        name: String,
        args: &[Arg],
        body: &[Stmt],
        is_async: bool,
        returns: &Option<Box<Expr>>,
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
        let mut new_code = CodeObject::new(&name);
        new_code.filename = self.code.filename.clone();
        let old_code = std::mem::replace(&mut self.code, new_code);
        let old_labels = std::mem::replace(&mut self.labels, Vec::new());
        // `new_label()` allocates ids by pushing to BOTH `self.labels` and
        // `self.label_positions` (compiler.rs), so resetting one without the
        // other lets a nested function's own labels collide with the
        // enclosing function's already-allocated ids: `self.labels` restarts
        // at length 0 here, but `label_positions` (if left alone) keeps
        // growing from wherever the enclosing scope left off — so id 0
        // means something different in each vector. A nested function's
        // `mark_label(0)` then silently overwrote the ENCLOSING loop's
        // `start_label` position with the nested function's own (much
        // smaller) instruction index, corrupting `emit_backward_jump`'s
        // computed offset the next time the enclosing loop wrapped around
        // (confirmed via `def outer(n):\n for i in range(n):\n  def f():\n
        // for j in range(2): yield j` — the loop jumped into the middle of
        // its OWN setup code instead of back to FOR_ITER, surfacing as
        // `TypeError: 'range_iterator' object is not callable` since a live
        // iterator ended up sitting where a callable was expected).
        let old_label_positions = std::mem::replace(&mut self.label_positions, Vec::new());
        let old_label_stack = std::mem::replace(&mut self.label_stack, Vec::new());
        let old_loop_stack = std::mem::replace(&mut self.loop_stack, Vec::new());
        let old_with_stack = std::mem::replace(&mut self.pending_cleanup, Vec::new());
        let old_current_line = self.current_line;
        self.current_line = 1;
        let old_annotations_initialized = self.annotations_initialized;
        self.annotations_initialized = false;

        self.enter_scope(ScopeType::Function);
        self.varnames_stack
            .push(Self::enclosing_snapshot(&old_code));

        // Pre-analyze the function to determine cell vars and free vars.
        // Use the nearest REAL enclosing function's varnames (skipping over
        // any intervening class-body scopes) so module globals aren't
        // treated as free vars, while methods nested inside a class body
        // can still see past it to the function that encloses the class.
        // PEP 3135: a method inside a class body additionally sees
        // `__class__` as a free var — the class body's own `__class__` cell,
        // populated by __build_class__ with the finished class, so bare
        // `super()` resolves the textually-enclosing class even when the
        // class is defined inside a function (its name isn't a global there).
        let mut enclosing_varnames = self.compute_enclosing_names();
        if self.scope == ScopeType::Function {
            if let Some(outer) = self.scope_stack.last() {
                if outer.scope == ScopeType::ClassBody {
                    enclosing_varnames.insert("__class__".to_string());
                }
            }
        }
        let (cell_vars, free_vars, local_names) = Self::analyze_function(
            args,
            body,
            &self.global_names,
            &self.nonlocal_names,
            Some(&enclosing_varnames),
        );
        self.code.cellvars = Box::new(cell_vars);
        self.code.freevars = Box::new(free_vars);

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
                self.code.vararg_name = Some(Box::new(self.mangle_name(&arg.arg)));
                if !arg_count_finalized {
                    self.code.arg_count = num_positional;
                    arg_count_finalized = true;
                }
                continue;
            }
            if arg.is_kwarg {
                self.code.kwarg_name = Some(Box::new(self.mangle_name(&arg.arg)));
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
        // Positional-only params (before `/` in the signature) are the FIRST
        // `posonlyarg_count` positional params.
        self.code.posonlyarg_count = args
            .iter()
            .take_while(|a| a.is_posonlyarg && !a.is_vararg && !a.is_kwarg && !a.is_kwonly)
            .count();

        // Add all args to varnames (including vararg/kwarg at the end) —
        // private names mangled so they match the body's mangled references.
        for arg in args {
            self.add_varname(&self.mangle_name(&arg.arg));
        }

        // Add cell vars to varnames too (so they get fast_locals slots)
        for cell_var in self.code.cellvars.clone().into_iter() {
            if self.get_var_index(&cell_var).is_none() {
                self.add_varname(&cell_var);
            }
        }

        // Pre-populate varnames with EVERY name this function assigns
        // anywhere in its own body (`local_names`, from the same upfront
        // `analyze_function` pass used for cell/free vars) — not just args
        // and cellvars. Without this, `Expr::Name`'s LOAD emission (which
        // decides LOAD_FAST vs LOAD_GLOBAL by checking whether `name` is
        // ALREADY in `self.code.varnames` at the point that particular
        // reference compiles) only saw whatever an incremental top-to-bottom
        // STORE_FAST pass had added SO FAR — a name referenced textually
        // BEFORE its first assignment in the same function scope (`def f():
        // print(x); x = 1`) wasn't in varnames yet, so it silently compiled
        // to LOAD_GLOBAL instead of LOAD_FAST. That's wrong two ways at
        // once: it can silently read an unrelated same-named GLOBAL instead
        // of correctly failing, and even when no such global exists it
        // raised plain `NameError` instead of `UnboundLocalError` (found via
        // CPython's own `test_scope.py::testUnboundLocal`, which explicitly
        // asserts `UnboundLocalError` for exactly this shape). Real Python's
        // static scoping rule is whole-function, not incremental: ANY
        // assignment anywhere in the function body makes that name local
        // for the ENTIRE body, including uses before the first assignment.
        for local_name in &local_names {
            if self.get_var_index(local_name).is_none() {
                self.add_varname(local_name);
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
        // Set CO_COROUTINE flag for async functions — both the public
        // CPython bit (0x80, what `inspect.CO_COROUTINE` exposes) and this
        // VM's internal 0x100 marker used by the frame driver.
        if is_async {
            self.code.flags |= 0x180;
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
        if std::env::var("RPY_DEBUG_COMPILE").is_ok()
            && std::env::var("RPY_DEBUG_COMPILE_NAME")
                .map(|n| n == name)
                .unwrap_or(false)
        {
            eprintln!(
                "  == {} instructions (cellvars={:?} freevars={:?} varnames={:?}) ==",
                name, inner_cell_vars, inner_free_vars, self.code.varnames
            );
            for (i, instr) in self.code.instructions.iter().enumerate() {
                eprintln!("    [{}] {:?} arg={}", i, instr.op, instr.arg);
            }
        }

        self.code.nlocals = self.code.varnames.len();
        self.code.name = crate::interner::intern(&name);
        self.code.first_lineno = old_current_line;

        self.code.cellvars = inner_cell_vars;
        self.code.freevars = inner_free_vars.clone();

        let func_code = std::mem::replace(&mut self.code, old_code);
        self.labels = old_labels;
        self.label_positions = old_label_positions;
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
                } // A name relayed from further out that we (as the
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

        let kwonly_defaults_count = args
            .iter()
            .filter(|a| a.is_kwonly && a.default.is_some())
            .count();
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

        // PEP 649 deferred annotations: build a nested `__annotate__`
        // function whose body RETURNS the annotations dict, and attach it as
        // `func.__annotate__`. The annotation EXPRESSIONS are only evaluated
        // when `__annotations__` is first accessed (real CPython 3.14
        // behavior) — eagerly evaluating them at def time broke modules with
        // annotations referencing names defined conditionally or later
        // (`Lib/_colorize.py`'s `-> Self`).
        let has_annotations = args.iter().any(|a| a.annotation.is_some()) || returns.is_some();
        if has_annotations {
            let mut keys: Vec<Option<Expr>> = Vec::new();
            let mut values: Vec<Expr> = Vec::new();
            for arg in args.iter() {
                if let Some(annotation) = &arg.annotation {
                    keys.push(Some(Expr::Constant(Constant::String(
                        self.mangle_name(&arg.arg),
                    ))));
                    values.push((**annotation).clone());
                }
            }
            if let Some(returns) = returns {
                keys.push(Some(Expr::Constant(Constant::String("return".to_string()))));
                values.push((**returns).clone());
            }
            let annotate_body = vec![Stmt::Return(Some(Box::new(Expr::Dict { keys, values })))];
            // Keep the main function on the stack for the attach below.
            self.emit(Opcode::DUP_TOP, 0);
            // The nested function needs its own docstring-free, no-defaults
            // signature; compile_function handles the recursion.
            self.compile_function(
                "__annotate__".to_string(),
                &[],
                &annotate_body,
                false,
                &None,
            )?;
            let annotate_attr_idx = self.get_name_index("__annotate__") as u32;
            self.emit(Opcode::STORE_ATTR, annotate_attr_idx);
        }

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

    /// Real Python's "private name mangling": any identifier textually
    /// occurring inside a class body that starts with two or more
    /// underscores and does NOT end with two or more (so `__dunder__` is
    /// exempt) is rewritten to `_ClassName__name` (leading underscores
    /// stripped from `ClassName` itself, per the language reference) before
    /// bytecode is generated for it. Was entirely unimplemented — `self.
    /// __x = 5` inside a method stored the literal, unmangled name `__x`
    /// in the instance dict instead of `_Foo__x`, so `instance._Foo__x`
    /// (the standard, extremely common way "private" attributes are
    /// accessed from outside — real trigger: CPython's own `test_binop.py`'s
    /// `Rat` class) raised `AttributeError` even though `self.__x` read
    /// back correctly FROM WITHIN the class (both the store and the load
    /// used the same un-mangled name, so they agreed with each other, just
    /// not with real Python's actual convention). Scoped to attribute
    /// access only (`self.__x`/`obj.__x`) — the by-far dominant real-world
    /// use of this feature — not bare local-variable-name mangling, which
    /// is a much rarer edge case.
    pub(crate) fn mangle_name(&self, name: &str) -> String {
        if let Some(class_name) = self.class_name_stack.last() {
            let looks_private = name.starts_with("__") && !name.ends_with("__");
            if looks_private {
                let stripped = class_name.trim_start_matches('_');
                if !stripped.is_empty() {
                    return format!("_{}{}", stripped, name);
                }
            }
        }
        name.to_string()
    }

    pub(crate) fn compile_class_body(&mut self, name: String, body: &[Stmt]) -> Result<(), String> {
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

        let mut new_class_code = CodeObject::new(&name);
        new_class_code.filename = self.code.filename.clone();
        let old_code = std::mem::replace(&mut self.code, new_class_code);
        self.varnames_stack
            .push(Self::enclosing_snapshot(&old_code));

        let old_labels = std::mem::replace(&mut self.labels, Vec::new());
        // See the matching comment in `compile_function` — `label_positions`
        // must be reset/restored in lockstep with `labels`, or a class body
        // compiled inside an enclosing loop can corrupt that loop's jump
        // targets the same way a nested `def` can.
        let old_label_positions = std::mem::replace(&mut self.label_positions, Vec::new());
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
        let (_ignored_cellvars, free_vars, _ignored_locals) = Self::analyze_function(
            &[],
            body,
            &self.global_names,
            &self.nonlocal_names,
            Some(&enclosing_varnames),
        );
        self.code.freevars = Box::new(free_vars);
        // PEP 3135: the class body itself owns the `__class__` cell that
        // `__build_class__` fills with the finished class — methods created
        // here reference it as a free var, so bare `super()` in them can
        // resolve the class. MUST be in cellvars (before the relay below
        // finalizes this scope's layout) so LOAD_CLOSURE/METHOD capture it,
        // AND in varnames so the frame's MAKE_CELL creates the cell.
        if !self.code.cellvars.contains(&"__class__".to_string()) {
            self.code.cellvars.push("__class__".to_string());
            if self.get_var_index("__class__").is_none() {
                self.add_varname("__class__");
            }
        }
        // The class body analysis may ALSO have marked `__class__` as one of
        // OUR freevars — because the method bodies nested here reference it
        // via `super()`, and the enclosing function's `__class__` (a
        // different cell: e.g. a class defined inside ANOTHER class's method
        // sees the OUTER class's `__class__`) is in our computed enclosing
        // names. That is wrong: this class body OWNS its own `__class__`
        // cell (pushed above), and methods must close over THAT one — the
        // relay prefers the freevar slot, which would thread the OUTER
        // class's cell through instead (bare `super()` then resolved to the
        // enclosing method's class, e.g. PositionalOnlyTestCase, and raised
        // AttributeError). Drop it from our freevars so the relay's cellvar
        // path is taken.
        if let Some(pos) = self.code.freevars.iter().position(|n| n == "__class__") {
            self.code.freevars.remove(pos);
        }

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

        // MAKE_CELL for the class body's cellvars (the `__class__` cell) so
        // the frame's fast_locals slot holds a real Cell for __build_class__
        // to populate.
        for cell_var in self.code.cellvars.clone().iter() {
            if let Some(idx) = self.get_var_index(cell_var) {
                self.emit(Opcode::MAKE_CELL, idx as u32);
            }
        }

        self.compile_stmts(body)?;

        let const_none = self.get_const_index(ConstValue::None) as u32;
        self.emit(Opcode::LOAD_CONST, const_none);
        self.emit(Opcode::RETURN_VALUE, 0);

        self.code.nlocals = self.code.varnames.len();
        self.code.name = crate::interner::intern(&name);
        self.code.first_lineno = old_current_line;

        let inner_free_vars = self.code.freevars.clone();

        let func_code = std::mem::replace(&mut self.code, old_code);
        self.labels = old_labels;
        self.label_positions = old_label_positions;
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

}
