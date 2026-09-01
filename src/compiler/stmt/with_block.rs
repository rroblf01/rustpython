use crate::ast::*;
use crate::bytecode::*;
use super::super::Compiler;
use super::super::scope::{LoopInfo, PendingCleanup, ScopeType};
use super::super::utils::delete_error_for;

impl Compiler {
    pub(crate) fn compile_with_stmt(&mut self, items: &[WithItem], body: &[Stmt], is_async: bool) -> Result<(), String> {
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
                    let mut rest = items.to_vec();
                    let first = rest.remove(0);
                    let inner = Stmt::With {
                        items: rest,
                        body: body.to_vec(),
                        is_async: is_async,
                    };
                    self.compile_stmt(&Stmt::With {
                        items: vec![first],
                        body: vec![inner],
                        is_async: is_async,
                    })?;
                    return Ok(());
                }
                let with_line = self.current_line;
                for (_i, item) in items.iter().enumerate() {
                    self.compile_expr(&item.context_expr)?;
                    if is_async {
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
                    self.pending_cleanup.push(PendingCleanup::With(is_async));
                    let with_result = self.compile_stmts(body);
                    self.pending_cleanup.pop();
                    with_result?;
                    self.emit(Opcode::POP_BLOCK, 0);
                    // Manager is still on the stack from SETUP_WITH
                    self.emit(Opcode::DUP_TOP, 0);
                    let exit_name = if is_async { "__aexit__" } else { "__exit__" };
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
                    let saved_line = self.current_line;
                    self.current_line = with_line;
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
                    // The exception was swallowed: POP_EXCEPT pops the
                    // exception_obj from the value stack AND restores the
                    // enclosing handler's active_exception and context-stack
                    // entry, so a later bare `raise` re-raises the OUTER
                    // exception (e.g. `except TypeError: with C(): raise K;
                    // raise` with `C.__exit__` returning True must re-raise
                    // TypeError). Must NOT POP_TOP first — POP_EXCEPT pops
                    // the value itself.
                    self.emit(Opcode::POP_EXCEPT, 0);
                    self.current_line = saved_line;
                    self.fix_label(end_label);
                } else {
                    self.compile_stmts(body)?;
                }
        Ok(())
    }

}
