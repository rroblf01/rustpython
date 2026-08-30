use crate::ast::*;
use crate::bytecode::*;
use super::super::Compiler;
use super::super::scope::{LoopInfo, PendingCleanup, ScopeType};
use super::super::utils::delete_error_for;

impl Compiler {
    /// Store the just-caught exception into `except E as name:`'s binding.
    /// Mirrors `compile_assign_target`'s own `Expr::Name` scope logic
    /// (STORE_NAME for module/class-body scope OR a name declared
    /// `global`, STORE_FAST otherwise) — this file used to check ONLY
    /// `self.scope == ScopeType::Module`, silently ignoring a `global`
    /// declaration. `global x` followed by `except E as x:` inside a
    /// function then stored into a throwaway local instead of the actual
    /// global, so `globals()["x"]` never saw it (test_global.py's
    /// `test_caught_exception`/`test_caught_exception_group`).
    fn store_except_name(&mut self, name: &str) {
        if self.scope == ScopeType::Module
            || self.scope == ScopeType::ClassBody
            || self.global_names.contains(name)
        {
            let name_idx = self.get_name_index(name) as u32;
            self.emit(Opcode::STORE_NAME, name_idx);
        } else {
            let idx = self.add_varname(name) as u32;
            self.emit(Opcode::STORE_FAST, idx);
        }
    }

    pub(crate) fn compile_try_stmt(&mut self, body: &[Stmt], handlers: &[ExceptHandler], handlers_star: &[ExceptStar], orelse: &[Stmt], finalbody: &[Stmt]) -> Result<(), String> {
                if !finalbody.is_empty()
                    && handlers.is_empty()
                    && handlers_star.is_empty()
                    && orelse.is_empty()
                {
                    // Simple try/finally
                    let finally_label = self.new_label();
                    let end_label = self.new_label();
                    self.emit_jump(Opcode::SETUP_FINALLY, finally_label);
                    // Tracked so a `return` compiled inside `body` knows to
                    // inline a copy of `finalbody` first — see
                    // `pending_cleanup`'s doc comment.
                    self.pending_cleanup
                        .push(PendingCleanup::Finally(finalbody.to_vec()));
                    let body_result = self.compile_stmts(body);
                    self.pending_cleanup.pop();
                    body_result?;
                    self.emit(Opcode::POP_BLOCK, 0);
                    self.compile_stmts(finalbody)?;
                    self.emit_jump(Opcode::JUMP, end_label);
                    self.fix_label(finally_label);
                    // arg=1 marks a `finally`-block entry (vs arg=0 for an
                    // `except` handler): the VM uses this to decide whether
                    // the pushed exception joins the PEP 3134 `__context__`
                    // stack. A finally block re-raises the SAME exception —
                    // it must NOT become the implicit context for raises made
                    // elsewhere; only the propagating-exception fallback in
                    // the VM applies during a finally body.
                    self.emit(Opcode::PUSH_EXC_INFO, 1);
                    self.compile_stmts(finalbody)?;
                    self.emit(Opcode::POP_EXCEPT, 1);
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
                    self.pending_cleanup
                        .push(PendingCleanup::Finally(finalbody.to_vec()));
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
                                self.store_except_name(name);
                            }
                            self.compile_stmts(&handler.body)?;
                            self.emit_jump(Opcode::JUMP, handler_done);
                            self.fix_label(next_handler);
                        } else {
                            if let Some(name) = &handler.name {
                                // See the identical `DUP_TOP` comment above.
                                self.emit(Opcode::DUP_TOP, 0);
                                self.store_except_name(name);
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
                                self.store_except_name(name);
                            } else {
                                self.emit(Opcode::POP_TOP, 0);
                            }
                            self.compile_stmts(&handler.body)?;
                            // Fall through to next handler (no JUMP to handler_done!)
                            self.fix_label(next_handler);
                        } else {
                            if let Some(name) = &handler.name {
                                self.store_except_name(name);
                            }
                            self.compile_stmts(&handler.body)?;
                        }
                    }
                    self.pending_cleanup.pop(); // PopExcept
                                                // except*: re-raise only the UNMATCHED exceptions. The
                                                // unmatched ExceptionGroup is on the value stack (pushed
                                                // by CHECK_EXC_MATCH_STAR); RERAISE must use it, not the
                                                // original exception still held in active_exception.
                    self.emit(Opcode::CLEAR_EXCEPTION_INFO, 0);
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
                    self.emit(Opcode::PUSH_EXC_INFO, 1);
                    self.compile_stmts(finalbody)?;
                    self.emit(Opcode::POP_EXCEPT, 1);
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
                                self.store_except_name(name);
                            }
                            self.compile_stmts(&handler.body)?;
                            self.emit_jump(Opcode::JUMP, handler_done);
                            self.fix_label(next_handler);
                        } else {
                            if let Some(name) = &handler.name {
                                // See the identical `DUP_TOP` comment above.
                                self.emit(Opcode::DUP_TOP, 0);
                                self.store_except_name(name);
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
                                self.store_except_name(name);
                            } else {
                                self.emit(Opcode::POP_TOP, 0);
                            }
                            self.compile_stmts(&handler.body)?;
                            // Fall through to next handler (no JUMP to handler_done!)
                            self.fix_label(next_handler);
                        } else {
                            if let Some(name) = &handler.name {
                                self.store_except_name(name);
                            }
                            self.compile_stmts(&handler.body)?;
                        }
                    }
                    self.pending_cleanup.pop(); // PopExcept
                                                // except*: re-raise only the UNMATCHED exceptions (see
                                                // the sibling except* block's comment).
                    self.emit(Opcode::CLEAR_EXCEPTION_INFO, 0);
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
        Ok(())
    }

}
