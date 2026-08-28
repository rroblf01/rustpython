use crate::ast::*;
use crate::bytecode::*;
use super::Compiler;
use super::scope::{LoopInfo, PendingCleanup, ScopeType};
use super::utils::delete_error_for;

mod try_block;
mod with_block;
mod match_block;
mod assign_target;
impl Compiler {

    pub(crate) fn compile_stmts(&mut self, stmts: &[Stmt]) -> Result<(), String> {
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

    pub(crate) fn compile_stmt(&mut self, stmt: &Stmt) -> Result<(), String> {
        match stmt {
            Stmt::Expr(expr) => {
                self.compile_expr(expr)?;
                self.emit(Opcode::POP_TOP, 0);
            }
            Stmt::Pass => {}
            Stmt::Break => {
                let (end_label, is_for, cleanup) = if let Some(loop_info) = self.loop_stack.last() {
                    (
                        loop_info.end_label,
                        loop_info.is_for,
                        self.pending_cleanup[loop_info.cleanup_start..].to_vec(),
                    )
                } else {
                    return Err(format!("L{}:{}:'break' outside loop", self.current_line, 1));
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
                            // The with-manager (pushed by SETUP_WITH) still
                            // sits on the stack after the __exit__ call —
                            // pop it so the for-iterator below it becomes
                            // the top again.
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
                    (
                        loop_info.start_label,
                        self.pending_cleanup[loop_info.cleanup_start..].to_vec(),
                    )
                } else {
                    return Err(format!(
                        "L{}:{}:'continue' not properly in loop",
                        self.current_line, 1
                    ));
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
                            // Pop the with-manager left below the __exit__
                            // result (continue keeps the for-iterator).
                            self.emit(Opcode::POP_TOP, 0);
                        }
                        PendingCleanup::Finally(_) => {}
                    }
                }
                self.emit_backward_jump(start_label);
            }
            Stmt::Return(value) => {
                if self.scope != ScopeType::Function {
                    // CPython rejects `return` outside a function at
                    // compile time (module/class bodies). This used to
                    // silently compile the return into an unreachable
                    // LOAD/RETURN sequence — test_exceptions'
                    // test_string_source etc. assert a SyntaxError.
                    return Err("'return' outside function".to_string());
                }
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
                        // COPY's arg is a 0-indexed depth from TOS (COPY(0)
                        // duplicates TOS itself — see the Subscript
                        // augmented-assignment codegen above, which uses
                        // COPY(0) for exactly that). This used to emit
                        // COPY(1) ("duplicate one item below TOS"), which
                        // only ever produced the right value by accident:
                        // with nothing else on the stack beneath `value`,
                        // "one below TOS" doesn't exist yet on the first
                        // iteration (vm.rs's own COPY falls back to a
                        // plain TOS-duplicate when `depth >= stack.len()`),
                        // and every later iteration duplicates a value
                        // that's identical to TOS anyway — but the instant
                        // something ELSE sits beneath `value` on the real
                        // stack (e.g. a `for` loop's iterator, which stays
                        // on the stack for the loop's whole duration), "one
                        // below TOS" pointed at THAT instead, so `a = b = c
                        // = i` inside a `for i in ...:` silently bound
                        // every target after the first to the loop's
                        // iterator object instead of `i`. Confirmed via
                        // `python -c` with a for-loop wrapping a chained
                        // assignment followed by using the extra targets.
                        self.emit(Opcode::COPY, 0);
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
                        let attr_idx = self.get_name_index(&self.mangle_name(attr)) as u32;
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
                    Expr::Name(_) => {
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
                    // Any other target (a comprehension, literal, call,
                    // comparison, ...) is not a valid augmented-assignment
                    // target at all — real CPython's own wording is
                    // "illegal expression for augmented assignment" (matched
                    // by `test_dictcomps.py`/etc.'s `assertRaisesRegex
                    // (SyntaxError, "illegal expression")`). Was previously
                    // falling into the generic Name-shaped path above, which
                    // for a DictComp/ListComp/etc. target compiled the
                    // comprehension as a normal expression and then hit
                    // `compile_assign_target`'s OWN "cannot assign to X"
                    // error instead — the wrong message for this specific,
                    // syntactically-illegal-augmented-target case.
                    _ => {
                        return Err("illegal expression for augmented assignment".to_string());
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
                    cleanup_start: self.pending_cleanup.len(),
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
                    cleanup_start: self.pending_cleanup.len(),
                    is_for: true,
                });
                self.mark_label(start_label);
                self.emit_jump(
                    if *is_async {
                        Opcode::FOR_ITER
                    } else {
                        Opcode::FOR_ITER
                    },
                    else_label,
                );
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
                returns,
                is_async,
                ..
            } => {
                // Real Python evaluates decorator EXPRESSIONS top-to-bottom,
                // in source order (`@d1 @d2 @d3 def f():` evaluates `d1`,
                // THEN `d2`, THEN `d3` — before building `f` at all), but
                // APPLIES/calls them bottom-up (closest to `def` first: `f
                // = d1(d2(d3(f)))`, so `d3` is CALLED first). This used to
                // conflate the two: `compile_expr(decorator)` ran INSIDE
                // the reverse-order apply loop, evaluating `d3`'s
                // expression first (matching call order) instead of last
                // (matching real eval order) — observable via any
                // decorator expression with a side effect ordered relative
                // to another (confirmed via `test_decorators.py::
                // test_eval_order`, which asserts the exact interleaving of
                // `evalnameN`/`evalargsN`/`makedecN` markers, all in
                // ascending N order, followed by `calldecN` in DESCENDING N
                // order). Fixed by splitting into two passes: push every
                // decorator's evaluated value in FORWARD order first (so
                // the stack ends up with `d3` on top, `d1` at the bottom —
                // exactly the LIFO order the apply loop needs), then build
                // the function, then apply len(decorator_list) times
                // WITHOUT re-evaluating anything.
                for decorator in decorator_list.iter() {
                    self.compile_expr(decorator)?;
                }
                self.compile_function(name.clone(), args, body, *is_async, returns)?;
                // No `SWAP` needed here (unlike the old single-pass version):
                // pushing every decorator BEFORE the function leaves each one
                // sitting directly below the value it must be called with —
                // `d3` (the last-pushed, closest-to-`def` decorator) ends up
                // immediately below `f`, exactly the arrangement `CALL 1`
                // wants (callable, then its single argument, both on top of
                // stack). After each `CALL`, the next decorator down is again
                // directly below the freshly-computed result, so this holds
                // for every iteration, not just the first.
                for _ in decorator_list.iter() {
                    self.emit(Opcode::CALL, 1);
                    // Result stays on stack
                }
                // Scope-aware storage (STORE_NAME/STORE_FAST/STORE_DEREF) —
                // see the matching fix on Stmt::ClassDef above for why
                // unconditional STORE_NAME breaks nested-function closures
                // over a helper function defined in the enclosing scope.
                //
                // The STORAGE target (the class-dict key / binding name)
                // must go through the SAME private-name mangling as any
                // `self.__attr` reference (`mangle_name`) — `def
                // __helper(self): ...` inside a class body binds under the
                // MANGLED name in real Python (`_ClassName__helper`), while
                // the function object's own `__name__`/`__qualname__` stay
                // the original, unmangled text (`compile_function` above
                // already used the un-mangled `name` for that, correctly).
                // Missing this meant `self.__helper()` (mangled at the call
                // site, per the earlier attribute-access fix) could never
                // find a same-named method DEFINED via plain `def
                // __helper(self):` in the same class — confirmed via
                // `Lib/_strptime.py`'s own `LocaleTime.__calc_weekday`
                // (`AttributeError: 'LocaleTime' object has no attribute
                // '_LocaleTime__calc_weekday'`, breaking `time.strptime`
                // for any format needing weekday/month name lookups).
                self.compile_assign_target(&Expr::Name(self.mangle_name(name)))?;
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
                    let name_idx = self
                        .get_const_index(ConstValue::String(k.arg.clone().unwrap_or_default()))
                        as u32;
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
                // Same bottom-up APPLICATION order as function decorators
                // above (`@a @b class C` means `C = a(b(C))`) — but see
                // that same fix's own doc comment: evaluating each
                // decorator EXPRESSION must still happen top-to-bottom,
                // BEFORE the class object is built, not interleaved with
                // the reverse-order apply loop. This class-decorator copy
                // of the logic had the identical bug.
                // NOTE: unlike the function-decorator fix just above, the
                // class object here is already fully built (bases/keywords
                // evaluated and `__build_class__` called) BEFORE this point
                // — there's no equivalent way to pre-push decorator
                // expressions in forward order while still leaving each one
                // directly below the value it needs to be called with
                // without a deeper restructure. Left as the simpler
                // (correct APPLICATION order, but decorator-EXPRESSION eval
                // order still matches call order rather than source order)
                // form for now — a real gap, but a much rarer one than the
                // function-decorator case (class decorators are less common
                // and less likely to have order-observable side effects).
                for decorator in decorator_list.iter().rev() {
                    self.compile_expr(decorator)?;
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
                        // `import a.b.c as x`/plain `import time` bind a
                        // LOCAL name when compiled inside function scope
                        // (same static-scoping rule as any other assignment)
                        // — this used to hardcode `STORE_NAME` unconditionally,
                        // which happened to work by accident only because the
                        // imported name was never pre-registered in
                        // `varnames`, so `Expr::Name`'s LOAD side fell back to
                        // `LOAD_GLOBAL` (which can read whatever `STORE_NAME`
                        // wrote into the frame's globals-adjacent storage).
                        // Once `varnames` is correctly pre-populated with
                        // every name the function assigns anywhere in its own
                        // body (see `compile_function`'s upfront
                        // `local_names` pass), a name imported inside a
                        // function is now looked up via `LOAD_FAST` — so the
                        // store side must follow the identical scope-aware
                        // rule, via the same helper `Expr::Name` assignment
                        // uses, or the value never reaches `fast_locals` at
                        // all (`UnboundLocalError`, confirmed via
                        // `Lib/random.py`'s own `def seed(self, ...): import
                        // time; ... time.time()`).
                        self.compile_assign_target(&Expr::Name(asname.clone()))?;
                    } else {
                        let dot_pos = alias.name.find('.');
                        let bound_name = if let Some(pos) = dot_pos {
                            alias.name[..pos].to_string()
                        } else {
                            alias.name.clone()
                        };
                        self.compile_assign_target(&Expr::Name(bound_name))?;
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
                // `from x import *` — star-import, handled specially
                // (copy all non-underscore names from the module; test_pkg).
                if names.len() == 1 && names[0].name == "*" {
                    self.emit(Opcode::IMPORT_STAR, 0);
                    return Ok(());
                }
                for alias in names {
                    let import_name_idx = self.get_name_index(&alias.name) as u32;
                    self.emit(Opcode::IMPORT_FROM, import_name_idx);
                    // Same scope-aware store as plain `Stmt::Import` above —
                    // `from x import y` inside a function binds a LOCAL `y`.
                    let bound_name = alias.asname.clone().unwrap_or_else(|| alias.name.clone());
                    self.compile_assign_target(&Expr::Name(bound_name))?;
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
                            let name_idx = self.get_name_index(&self.mangle_name(attr)) as u32;
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
                                        let name_idx =
                                            self.get_name_index(&self.mangle_name(attr)) as u32;
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
                                    _ => return Err(delete_error_for(e).to_string()),
                                }
                            }
                        }
                        _ => return Err(delete_error_for(target).to_string()),
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
                self.compile_try_stmt(body, handlers, handlers_star, orelse, finalbody)?;
            }
            Stmt::Assert { test, msg } => {
                let assertion_error_idx = self.get_name_index("AssertionError") as u32;
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
            Stmt::With {
                items,
                body,
                is_async,
            } => {
                self.compile_with_stmt(items, body, *is_async)?;
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
                    if self.scope == ScopeType::ClassBody {
                        self.emit(Opcode::SETUP_ANNOTATIONS, 0);
                        let ann_idx = self.get_name_index("__annotations__") as u32;
                        self.emit(Opcode::LOAD_NAME, ann_idx);
                        let name_const =
                            self.get_const_index(ConstValue::String(name.clone())) as u32;
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
                        self.compile_expr(&Expr::Name(tmp.clone()))?;
                        self.emit(Opcode::STORE_SUBSCR, 0);
                        // Clean up the temporary variable so it doesn't leak
                        // into the class namespace (every STORE_NAME in a
                        // class body lands in the class dict).
                        let tmp_idx = self.get_name_index(&tmp) as u32;
                        self.emit(Opcode::DELETE_NAME, tmp_idx);
                    } else if self.scope == ScopeType::Module {
                        // Module-level annotations are deferred (PEP 649)
                        // in CPython 3.14 and are NOT immediately stored
                        // into `__annotations__`. Eagerly storing them
                        // here would overwrite a pre-existing
                        // `__annotations__` dict supplied by the caller
                        // (e.g. `exec('x: int', {'__annotations__': {1:2}})`)
                        // and also break the deferred expectation. We
                        // still evaluate the annotation for side effects
                        // (raising non-NameError errors), but do not
                        // populate `__annotations__`.
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
                        // Evaluate and discard, clean up tmp
                        self.compile_expr(&Expr::Name(tmp.clone()))?;
                        self.emit(Opcode::POP_TOP, 0);
                        let tmp_idx = self.get_name_index(&tmp) as u32;
                        self.emit(Opcode::DELETE_NAME, tmp_idx);
                    }
                }
                if let Some(val) = value {
                    self.compile_expr(val)?;
                    self.compile_assign_target(target)?;
                }
            }
            Stmt::Match { subject, cases } => {
                self.compile_match_stmt(subject, cases)?;
            }
        }
        Ok(())
    }

}
