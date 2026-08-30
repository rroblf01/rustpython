use crate::ast::*;
use crate::bytecode::*;
use super::super::Compiler;
use super::super::scope::{LoopInfo, PendingCleanup, ScopeType};
use super::super::utils::delete_error_for;

impl Compiler {
    /// Store a name captured by a `match`/`case` pattern (`case [x]:`,
    /// `case Point(x=x):`, `case {"k": v}:`, `case [*rest]:`, ...).
    /// Every call site here used to hardcode STORE_FAST unconditionally,
    /// ignoring module/class-body scope and `global` declarations
    /// entirely — same bug shape as `try_block.rs`'s `except E as name:`
    /// (see `store_except_name`'s doc comment), just never fixed here.
    fn emit_match_store(&mut self, name: &str) {
        if self.scope == ScopeType::Module
            || self.scope == ScopeType::ClassBody
            || self.global_names.contains(name)
        {
            let idx = self.get_name_index(name) as u32;
            self.emit(Opcode::STORE_NAME, idx);
        } else {
            let idx = self.add_varname(name) as u32;
            self.emit(Opcode::STORE_FAST, idx);
        }
    }

    pub(crate) fn compile_match_stmt(&mut self, subject: &Expr, cases: &[MatchCase]) -> Result<(), String> {
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
                            self.emit(Opcode::DUP_TOP, 0);
                            self.emit_match_store(n);
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
                            let star_index = patterns
                                .iter()
                                .position(|p| matches!(p, Pattern::MatchStar { .. }));
                            self.emit(Opcode::DUP_TOP, 0);
                            // Get length of subject
                            let len_name_idx = self.get_name_index("len") as u32;
                            self.emit(Opcode::LOAD_GLOBAL, len_name_idx);
                            self.emit(Opcode::SWAP, 1);
                            self.emit(Opcode::CALL, 1);
                            if let Some(si) = star_index {
                                // With MatchStar: require len(subject) >= patterns.len() - 1
                                let min_len = patterns.len() - 1;
                                let length_const = self
                                    .get_const_index(ConstValue::Int(min_len.to_string()))
                                    as u32;
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
                                            self.emit_match_store(n);
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
                                            })
                                                as u32;
                                            self.emit(Opcode::LOAD_CONST, const_idx);
                                            self.emit(Opcode::COMPARE_OP, 8); // IS
                                            self.emit_jump(Opcode::POP_JUMP_IF_FALSE, next_case);
                                        }
                                        Pattern::MatchStar { name } => match name {
                                            Some(n) => {
                                                self.emit_match_store(n);
                                            }
                                            None => {
                                                self.emit(Opcode::POP_TOP, 0);
                                            }
                                        },
                                        _ => {
                                            self.emit(Opcode::POP_TOP, 0);
                                        }
                                    }
                                }
                            } else {
                                // No MatchStar: exact length check + sequential extraction
                                let length_const = self
                                    .get_const_index(ConstValue::Int(patterns.len().to_string()))
                                    as u32;
                                self.emit(Opcode::LOAD_CONST, length_const);
                                self.emit(Opcode::COMPARE_OP, 2); // ==
                                self.emit_jump(Opcode::POP_JUMP_IF_FALSE, next_case);
                                for (i, pat) in patterns.iter().enumerate() {
                                    let idx_const =
                                        self.get_const_index(ConstValue::Int(i.to_string())) as u32;
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
                                            self.emit_match_store(n);
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
                                            })
                                                as u32;
                                            self.emit(Opcode::LOAD_CONST, const_idx);
                                            self.emit(Opcode::COMPARE_OP, 8); // IS
                                            self.emit_jump(Opcode::POP_JUMP_IF_FALSE, next_case);
                                        }
                                        _ => {
                                            self.emit(Opcode::POP_TOP, 0);
                                        }
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
                                        _ => {
                                            return Err(
                                                "Mapping pattern keys must be literal values"
                                                    .to_string(),
                                            )
                                        }
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
                                            self.emit_match_store(n);
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
                                            })
                                                as u32;
                                            self.emit(Opcode::LOAD_CONST, const_idx);
                                            self.emit(Opcode::COMPARE_OP, 8); // IS
                                            self.emit_jump(Opcode::POP_JUMP_IF_FALSE, next_case);
                                        }
                                        Pattern::MatchSequence(..) => {
                                            self.emit(Opcode::DUP_TOP, 0);
                                            let list_idx = self.get_const_index(ConstValue::String(
                                                "list".to_string(),
                                            ))
                                                as u32;
                                            self.emit(Opcode::LOAD_CONST, list_idx);
                                            self.emit(Opcode::CONTAINS_OP, 1);
                                            let seq_ok = self.new_label();
                                            self.emit_jump(Opcode::POP_JUMP_IF_TRUE, seq_ok);
                                            let tuple_idx = self.get_const_index(
                                                ConstValue::String("tuple".to_string()),
                                            )
                                                as u32;
                                            self.emit(Opcode::LOAD_CONST, tuple_idx);
                                            self.emit(Opcode::CONTAINS_OP, 1);
                                            self.emit_jump(Opcode::POP_JUMP_IF_FALSE, next_case);
                                            self.emit_label(seq_ok);
                                            self.emit(Opcode::POP_TOP, 0);
                                        }
                                        Pattern::MatchMapping { .. } => {
                                            self.emit(Opcode::DUP_TOP, 0);
                                            let dict_idx = self.get_const_index(ConstValue::String(
                                                "dict".to_string(),
                                            ))
                                                as u32;
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
                                                        self.emit_jump(
                                                            Opcode::POP_JUMP_IF_FALSE,
                                                            try_next,
                                                        );
                                                        self.emit_jump(Opcode::JUMP, or_matched);
                                                    }
                                                    Pattern::MatchAs { name: Some(n), .. } => {
                                                        self.emit_match_store(n);
                                                        self.emit_jump(Opcode::JUMP, or_matched);
                                                    }
                                                    _ => {
                                                        self.emit(Opcode::POP_TOP, 0);
                                                    }
                                                }
                                                self.emit_label(try_next);
                                            }
                                            self.emit_jump(Opcode::JUMP, next_case);
                                            self.emit_label(or_matched);
                                            self.emit(Opcode::POP_TOP, 0);
                                        }
                                        _ => {
                                            return Err(
                                                "Mapping sub-pattern not supported".to_string()
                                            )
                                        }
                                    }
                                }
                            }
                            // Handle **rest capture
                            if let Some(rest_name) = rest {
                                self.emit(Opcode::DUP_TOP, 0);
                                self.emit_match_store(rest_name);
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
                                    self.emit_match_store(n);
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
                                        self.emit_match_store(n);
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
                                        })
                                            as u32;
                                        self.emit(Opcode::LOAD_CONST, const_idx);
                                        self.emit(Opcode::COMPARE_OP, 8); // IS
                                        self.emit_jump(Opcode::POP_JUMP_IF_FALSE, try_next);
                                        self.emit_jump(Opcode::JUMP, or_matched);
                                        self.fix_label(try_next);
                                    }
                                    Pattern::MatchClass {
                                        cls,
                                        patterns,
                                        kwd_attrs,
                                        kwd_patterns,
                                    } => {
                                        // Same isinstance-then-subpattern check as the
                                        // plain (non-Or) MatchClass arm below, but a
                                        // failure falls through to the next alternative
                                        // (try_next) instead of the next case.
                                        self.emit(Opcode::DUP_TOP, 0);
                                        let try_next = self.new_label();
                                        let isinstance_idx =
                                            self.get_name_index("isinstance") as u32;
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
                                                    self.emit_jump(
                                                        Opcode::POP_JUMP_IF_FALSE,
                                                        try_next,
                                                    );
                                                }
                                                Pattern::MatchAs { name: Some(n), .. } => {
                                                    self.emit_match_store(n);
                                                }
                                                Pattern::MatchAs { name: None, .. } => {
                                                    self.emit(Opcode::POP_TOP, 0);
                                                }
                                                _ => {
                                                    self.emit(Opcode::POP_TOP, 0);
                                                }
                                            }
                                        }
                                        for (kwd_attr, kwd_pat) in
                                            kwd_attrs.iter().zip(kwd_patterns.iter())
                                        {
                                            self.emit(Opcode::DUP_TOP, 0);
                                            let attr_idx = self.get_name_index(kwd_attr) as u32;
                                            self.emit(Opcode::LOAD_ATTR, attr_idx);
                                            match kwd_pat {
                                                Pattern::MatchValue(val) => {
                                                    self.compile_expr(val)?;
                                                    self.emit(Opcode::COMPARE_OP, 2);
                                                    self.emit_jump(
                                                        Opcode::POP_JUMP_IF_FALSE,
                                                        try_next,
                                                    );
                                                }
                                                Pattern::MatchAs { name: Some(n), .. } => {
                                                    self.emit_match_store(n);
                                                }
                                                Pattern::MatchAs { name: None, .. } => {
                                                    self.emit(Opcode::POP_TOP, 0);
                                                }
                                                _ => {
                                                    self.emit(Opcode::POP_TOP, 0);
                                                }
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
                        Pattern::MatchClass {
                            cls,
                            patterns,
                            kwd_attrs,
                            kwd_patterns,
                        } => {
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
                                        self.emit_match_store(n);
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
                                        })
                                            as u32;
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
                                        self.emit_match_store(n);
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
                                        })
                                            as u32;
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
        Ok(())
    }

}
