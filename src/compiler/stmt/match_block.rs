use super::super::scope::{LoopInfo, PendingCleanup, ScopeType};
use super::super::utils::delete_error_for;
use super::super::Compiler;
use crate::ast::*;
use crate::bytecode::*;

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

    /// Shared mapping-pattern matcher used by both the plain
    /// `case {...}:` arm (fail = next case) and `Or` alternatives
    /// (fail = next alternative). Operates on the TOP of stack:
    /// entry [.., X], success exit [.., X], EVERY failure exit [.., X]
    /// — the caller owns X (drops it or jumps with it).
    fn compile_mapping_match(
        &mut self,
        keys: &[Pattern],
        rest: &Option<String>,
        fail: usize,
    ) -> Result<(), String> {
        // Entry gate: a non-mapping subject (int, list, ...) must fail
        // the case, not raise inside CONTAINS_OP/SUBSCR below.
        self.emit(Opcode::MATCH_MAPPING, 0);
        self.emit_jump(Opcode::POP_JUMP_IF_FALSE, fail);
        // keys are interleaved: [key1_pat, val1_pat, key2_pat, val2_pat, ...]
        // Collect the key expressions first: literal duplicates are a
        // static SyntaxError (parser validator), but ATTRIBUTE keys can
        // alias at runtime (`{Keys.KEY: y, "a": z}` with KEY = "a") —
        // CPython raises ValueError from its match-keys loop, so each
        // key is compared against all previous ones here.
        let mut key_exprs: Vec<&Expr> = Vec::new();
        for chunk in keys.chunks(2) {
            if let [key_pat, _] = chunk {
                match key_pat {
                    Pattern::MatchValue(expr) => key_exprs.push(expr),
                    _ => {
                        return Err("Mapping pattern keys must be literal values".to_string());
                    }
                }
            }
        }
        // Runtime duplicate-key check for ATTRIBUTE keys (literal
        // duplicates are a static SyntaxError). CPython raises ValueError
        // from its match-keys loop BEFORE binding any value (`y` stays
        // None in `{Keys.KEY: y, "a": z}`), so ALL pairs are compared up
        // front, before the per-key extraction loop below binds anything.
        // (Keys re-evaluate per comparison — CPython evaluates once into
        // its keys tuple; pattern keys are literals or attribute loads,
        // pure in practice.)
        for (i, key_i) in key_exprs.iter().enumerate() {
            for prev in key_exprs.iter().take(i) {
                self.compile_expr(key_i)?;
                self.compile_expr(prev)?;
                self.emit(Opcode::COMPARE_OP, 2); // ==
                let no_dup = self.new_label();
                self.emit_jump(Opcode::POP_JUMP_IF_FALSE, no_dup);
                let msg_idx = self.get_const_index(ConstValue::String(
                    "mapping pattern checks duplicate key".into(),
                )) as u32;
                let ve_idx = self.get_name_index("ValueError") as u32;
                self.emit(Opcode::LOAD_GLOBAL, ve_idx);
                self.emit(Opcode::LOAD_CONST, msg_idx);
                self.emit(Opcode::CALL, 1);
                self.emit(Opcode::RAISE_VARARGS, 1);
                self.fix_label(no_dup);
            }
        }
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
                self.emit_jump(Opcode::POP_JUMP_IF_FALSE, fail);
                // Get value: subject[key]
                self.emit(Opcode::DUP_TOP, 0);
                self.compile_expr(key_expr)?;
                self.emit(Opcode::BINARY_OP, 13); // BINARY_SUBSCR
                                                  // Match value against pattern
                match val_pat {
                    Pattern::MatchValue(val) => {
                        self.compile_expr(val)?;
                        self.emit(Opcode::COMPARE_OP, 2); // ==
                        self.emit_jump(Opcode::POP_JUMP_IF_FALSE, fail);
                    }
                    Pattern::MatchAs {
                        pattern: Some(inner),
                        name: Some(n),
                    } => {
                        self.compile_as_value_match(inner, n, fail)?;
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
                            _ => ConstValue::String(s.clone().into()),
                        }) as u32;
                        self.emit(Opcode::LOAD_CONST, const_idx);
                        self.emit(Opcode::COMPARE_OP, 8); // IS
                        self.emit_jump(Opcode::POP_JUMP_IF_FALSE, fail);
                    }
                    Pattern::MatchSequence(..) | Pattern::MatchMapping { .. } => {
                        // Nested sequence/mapping values recurse
                        // element-level (lengths checked, subpatterns
                        // bound) through the value checker — entry
                        // [X, v], all exits [X]. The old gate-only arms
                        // accepted any sequence/mapping here without
                        // checking contents (PATMA-032: `{0: [1, 2]}`
                        // matched a 3-tuple).
                        self.compile_value_check(val_pat, fail)?;
                    }
                    Pattern::MatchOr(subpatterns) => {
                        let or_matched = self.new_label();
                        let mut flat: Vec<&Pattern> = Vec::new();
                        flatten_or_patterns(subpatterns, &mut flat);
                        for subpat in flat {
                            self.emit(Opcode::DUP_TOP, 0);
                            let try_next = self.new_label();
                            match subpat {
                                Pattern::MatchValue(val) => {
                                    self.compile_expr(val)?;
                                    self.emit(Opcode::COMPARE_OP, 2);
                                    self.emit_jump(Opcode::POP_JUMP_IF_FALSE, try_next);
                                    self.emit_jump(Opcode::JUMP, or_matched);
                                }
                                Pattern::MatchAs {
                                    pattern: None,
                                    name: Some(n),
                                } => {
                                    self.emit_match_store(n);
                                    self.emit_jump(Opcode::JUMP, or_matched);
                                }
                                Pattern::MatchAs {
                                    pattern: None,
                                    name: None,
                                } => {
                                    // Bare `_` alternative always matches.
                                    self.emit(Opcode::POP_TOP, 0);
                                    self.emit_jump(Opcode::JUMP, or_matched);
                                }
                                Pattern::MatchAs {
                                    pattern: Some(inner),
                                    name: Some(n),
                                } => {
                                    // `2 as z`: check the inner pattern
                                    // first, bind only on match (PATMA-028:
                                    // the old code bound unconditionally).
                                    // Entry (loop-dup'd) [X,v,v]: the check
                                    // consumes one copy; re-dup for the
                                    // store so or_matched's POP lands on [X].
                                    self.compile_value_check(inner, try_next)?;
                                    self.emit(Opcode::DUP_TOP, 0);
                                    self.emit_match_store(n);
                                    self.emit_jump(Opcode::JUMP, or_matched);
                                }
                                Pattern::MatchClass {
                                    cls,
                                    patterns,
                                    kwd_attrs,
                                    kwd_patterns,
                                } => {
                                    // Class alternative: the funnel sheds
                                    // its own copy ([X,v,v] -> [X,v]) and
                                    // jumps class_fail with exactly that on
                                    // a miss — which is precisely what the
                                    // next alternative's DUP expects, so
                                    // fall straight into try_next.
                                    let class_fail = self.new_label();
                                    self.compile_class_value_match(
                                        cls,
                                        patterns,
                                        kwd_attrs,
                                        kwd_patterns,
                                        class_fail,
                                    )?;
                                    self.emit_jump(Opcode::JUMP, or_matched);
                                    self.fix_label(class_fail);
                                }
                                _ => {
                                    self.emit(Opcode::POP_TOP, 0);
                                }
                            }
                            self.fix_label(try_next);
                        }
                        // All alternatives failed: drop the leftover value
                        // so the exit shape stays [X] like every other
                        // failure path (a bare JUMP here used to leak the
                        // value, corrupting later cases into underflows).
                        self.emit(Opcode::POP_TOP, 0);
                        self.emit_jump(Opcode::JUMP, fail);
                        self.fix_label(or_matched);
                        self.emit(Opcode::POP_TOP, 0);
                    }
                    Pattern::MatchStar { name } => match name {
                        // `_` never binds (CPython wildcard),
                        // even though the parser keeps it as
                        // `Some("_")`.
                        Some(n) if n != "_" => {
                            self.emit_match_store(n);
                        }
                        _ => {
                            self.emit(Opcode::POP_TOP, 0);
                        }
                    },
                    Pattern::MatchClass {
                        cls,
                        patterns,
                        kwd_attrs,
                        kwd_patterns,
                    } => {
                        // Class value (`{"k": Point(x)}`): full funnel,
                        // same [..., v]/[...] contract as every arm here.
                        self.compile_class_value_match(
                            cls,
                            patterns,
                            kwd_attrs,
                            kwd_patterns,
                            fail,
                        )?;
                    }
                    _ => return Err("Mapping sub-pattern not supported".to_string()),
                }
            }
        }
        // Handle **rest capture: a NEW dict holding only the
        // unmatched keys (CPython MATCH_KEYS semantics) — storing X
        // itself used to bind the whole subject (PATMA-195/196/...).
        if let Some(rest_name) = rest {
            self.emit(Opcode::DUP_TOP, 0); // [X, X]
            let dict_idx = self.get_name_index("dict") as u32;
            self.emit(Opcode::LOAD_GLOBAL, dict_idx); // [X, X, dict]
            self.emit(Opcode::SWAP, 1); // [X, dict, X]
            self.emit(Opcode::CALL, 1); // [X, Xcopy]
            for chunk in keys.chunks(2) {
                if let [key_pat, _] = chunk {
                    let key_expr = match key_pat {
                        Pattern::MatchValue(expr) => expr,
                        _ => {
                            return Err("Mapping pattern keys must be literal values".to_string());
                        }
                    };
                    self.emit(Opcode::DUP_TOP, 0); // [X, Xc, Xc]
                    self.compile_expr(key_expr)?; // [X, Xc, Xc, k]
                    self.emit(Opcode::DELETE_SUBSCR, 0); // [X, Xc]
                }
            }
            self.emit_match_store(rest_name); // [X]
        }
        Ok(())
    }

    /// Exact-length sequence matcher (no `*star`), same contract as
    /// `compile_mapping_match`: entry [.., X], all exits [.., X].
    /// Only the no-star shape qualifies — the UNPACK_EX star path leaves
    /// leftovers on failure, so `Or` alternatives with `*star` fall back
    /// to lenient non-match instead of calling this.
    fn compile_sequence_exact_match(
        &mut self,
        patterns: &[Pattern],
        fail: usize,
    ) -> Result<(), String> {
        // Gate before duplicating (a dup'd copy would leak on failure).
        self.emit(Opcode::MATCH_SEQUENCE, 0);
        self.emit_jump(Opcode::POP_JUMP_IF_FALSE, fail);
        self.emit(Opcode::DUP_TOP, 0);
        // len(subject), same shape as the star path in the plain arm.
        let len_name_idx = self.get_name_index("len") as u32;
        self.emit(Opcode::LOAD_GLOBAL, len_name_idx);
        self.emit(Opcode::SWAP, 1);
        self.emit(Opcode::CALL, 1);
        let length_const = self.get_const_index(ConstValue::Int(patterns.len().to_string())) as u32;
        self.emit(Opcode::LOAD_CONST, length_const);
        self.emit(Opcode::COMPARE_OP, 2); // ==
        self.emit_jump(Opcode::POP_JUMP_IF_FALSE, fail);
        for (i, pat) in patterns.iter().enumerate() {
            let idx_const = self.get_const_index(ConstValue::Int(i.to_string())) as u32;
            self.emit(Opcode::DUP_TOP, 0);
            self.emit(Opcode::LOAD_CONST, idx_const);
            self.emit(Opcode::BINARY_OP, 13); // BINARY_SUBSCR
                                              // Every element kind (values, captures, singletons, nested
                                              // sequences/mappings/classes/or-patterns) goes through the
                                              // value checker: entry [.., X, elem], all exits [.., X].
            self.compile_value_check(pat, fail)?;
        }
        Ok(())
    }

    /// Check a value-position pattern against the value on top of stack.
    /// Entry [..., V], success exit [...], failure jumps to `fail` with [...].
    /// Sequence/mapping/class subpatterns recurse element-level (with
    /// bindings); only `*star` sequences stay gate-only (the UNPACK_EX
    /// star path leaves leftovers on failure).
    fn compile_value_check(&mut self, pat: &Pattern, fail: usize) -> Result<(), String> {
        match pat {
            Pattern::MatchValue(val) => {
                self.compile_expr(val)?;
                self.emit(Opcode::COMPARE_OP, 2); // ==
                self.emit_jump(Opcode::POP_JUMP_IF_FALSE, fail);
                Ok(())
            }
            Pattern::MatchSingleton(s) => {
                let const_idx = self.get_const_index(match s.as_str() {
                    "None" => ConstValue::None,
                    "True" => ConstValue::Bool(true),
                    "False" => ConstValue::Bool(false),
                    _ => ConstValue::String(s.clone().into()),
                }) as u32;
                self.emit(Opcode::LOAD_CONST, const_idx);
                self.emit(Opcode::COMPARE_OP, 8); // IS
                self.emit_jump(Opcode::POP_JUMP_IF_FALSE, fail);
                Ok(())
            }
            Pattern::MatchAs {
                pattern: None,
                name: Some(n),
            } => {
                self.emit_match_store(n);
                Ok(())
            }
            Pattern::MatchAs {
                pattern: None,
                name: None,
            } => {
                self.emit(Opcode::POP_TOP, 0);
                Ok(())
            }
            Pattern::MatchAs {
                pattern: Some(inner),
                name: Some(n),
            } => {
                // Check against a copy; the copy must be shed on BOTH
                // paths (a bare inner jump to `fail` used to leak it,
                // so `case (P | Q as z)`-shaped failures corrupted the
                // stack for every later case).
                self.emit(Opcode::DUP_TOP, 0);
                let inner_fail = self.new_label();
                let done = self.new_label();
                self.compile_value_check(inner, inner_fail)?;
                self.emit_match_store(n);
                self.emit_jump(Opcode::JUMP, done);
                self.fix_label(inner_fail);
                self.emit(Opcode::POP_TOP, 0);
                self.emit_jump(Opcode::JUMP, fail);
                self.fix_label(done);
                Ok(())
            }
            Pattern::MatchAs {
                pattern: Some(inner),
                name: None,
            } => {
                self.emit(Opcode::DUP_TOP, 0);
                let inner_fail = self.new_label();
                let done = self.new_label();
                self.compile_value_check(inner, inner_fail)?;
                self.emit(Opcode::POP_TOP, 0);
                self.emit_jump(Opcode::JUMP, done);
                self.fix_label(inner_fail);
                self.emit(Opcode::POP_TOP, 0);
                self.emit_jump(Opcode::JUMP, fail);
                self.fix_label(done);
                Ok(())
            }
            Pattern::MatchOr(alts) => {
                let mut flat: Vec<&Pattern> = Vec::new();
                flatten_or_patterns(alts, &mut flat);
                let done = self.new_label();
                for alt in flat {
                    // Entry [V]: dup a copy for the check; success and
                    // failure both leave [V] for the next step.
                    self.emit(Opcode::DUP_TOP, 0);
                    let inner_next = self.new_label();
                    self.compile_value_check(alt, inner_next)?;
                    self.emit_jump(Opcode::JUMP, done);
                    self.fix_label(inner_next);
                }
                // All alternatives failed ([X,V] on stack): shed the
                // value so the failure exit stays [...] like every other
                // arm (callers' miss trampolines pop exactly one slot
                // per level — a leaked value here corrupted them).
                self.emit(Opcode::POP_TOP, 0);
                self.emit_jump(Opcode::JUMP, fail);
                self.fix_label(done);
                // Matched: drop the leftover value ([V] -> []).
                self.emit(Opcode::POP_TOP, 0);
                Ok(())
            }
            Pattern::MatchSequence(patterns)
                if !patterns
                    .iter()
                    .any(|p| matches!(p, Pattern::MatchStar { .. })) =>
            {
                // Starless sequences recurse element-level (with
                // bindings) through the exact matcher — whose contract
                // keeps X ([.., X] in AND out), while this one consumes
                // V ([..., V] -> [...]). The trampoline bridges both
                // shapes (success sheds X; misses shed X then jump).
                let inner_fail = self.new_label();
                let done = self.new_label();
                self.compile_sequence_exact_match(patterns, inner_fail)?;
                self.emit(Opcode::POP_TOP, 0);
                self.emit_jump(Opcode::JUMP, done);
                self.fix_label(inner_fail);
                self.emit(Opcode::POP_TOP, 0);
                self.emit_jump(Opcode::JUMP, fail);
                self.fix_label(done);
                Ok(())
            }
            Pattern::MatchSequence(..) => {
                self.emit(Opcode::MATCH_SEQUENCE, 0);
                self.emit_jump(Opcode::POP_JUMP_IF_FALSE, fail);
                self.emit(Opcode::POP_TOP, 0);
                Ok(())
            }
            Pattern::MatchMapping { keys, rest } => {
                // Same shape bridge as sequences: the mapping matcher
                // keeps X, this contract consumes V.
                let inner_fail = self.new_label();
                let done = self.new_label();
                self.compile_mapping_match(keys, rest, inner_fail)?;
                self.emit(Opcode::POP_TOP, 0);
                self.emit_jump(Opcode::JUMP, done);
                self.fix_label(inner_fail);
                self.emit(Opcode::POP_TOP, 0);
                self.emit_jump(Opcode::JUMP, fail);
                self.fix_label(done);
                Ok(())
            }
            Pattern::MatchClass {
                cls,
                patterns,
                kwd_attrs,
                kwd_patterns,
            } => {
                // Nested class patterns get the full funnel (extraction
                // + bindings), not just the isinstance gate.
                self.compile_class_value_match(cls, patterns, kwd_attrs, kwd_patterns, fail)?;
                Ok(())
            }
            Pattern::MatchStar { name } => match name {
                Some(n) if n != "_" => {
                    self.emit_match_store(n);
                    Ok(())
                }
                _ => {
                    self.emit(Opcode::POP_TOP, 0);
                    Ok(())
                }
            },
        }
    }

    /// Full class-pattern check for a value on top of stack, for nesting
    /// inside sequences/mappings/or-alternatives/as-patterns.
    /// Entry [..., V]; success [...] (V consumed, subpatterns checked and
    /// bound); failure jumps `fail` with [...] (V and the class shed).
    /// The class compiles once; CHECK runs the CPython `_PyEval_MatchClass`
    /// order (not-a-class TypeError -> isinstance -> __match_args__
    /// validation, the last two only ... validation only on match).
    fn compile_class_value_match(
        &mut self,
        cls: &Expr,
        patterns: &[Pattern],
        kwd_attrs: &[Ident],
        kwd_patterns: &[Pattern],
        fail: usize,
    ) -> Result<(), String> {
        self.compile_expr(cls)?; // [..., V, C]
        let kwd_tuple_idx = self.get_const_index(ConstValue::Tuple(kwd_attrs.to_vec())) as u32;
        self.emit(Opcode::LOAD_CONST, kwd_tuple_idx); // [..., V, C, kwd]
        self.emit(Opcode::MATCH_CLASS_CHECK, patterns.len() as u32); // [...,V,C,bool]
        let inner_fail = self.new_label();
        let done = self.new_label();
        self.emit_jump(Opcode::POP_JUMP_IF_FALSE, inner_fail); // [..., V, C]
        for (i, pat) in patterns.iter().enumerate() {
            self.emit(Opcode::MATCH_CLASS_GETATTR, i as u32); // [..., V, C, a]
            self.compile_value_check(pat, inner_fail)?; // [..., V, C]
        }
        for (kwd_attr, kwd_pat) in kwd_attrs.iter().zip(kwd_patterns.iter()) {
            self.emit(Opcode::COPY, 1); // [..., V, C, V]
            let attr_idx = self.get_name_index(kwd_attr) as u32;
            self.emit(Opcode::LOAD_ATTR, attr_idx); // [..., V, C, a]
            self.compile_value_check(kwd_pat, inner_fail)?; // [..., V, C]
        }
        self.emit(Opcode::POP_TOP, 0); // [..., V]
        self.emit(Opcode::POP_TOP, 0); // [...]
        self.emit_jump(Opcode::JUMP, done);
        self.fix_label(inner_fail);
        self.emit(Opcode::POP_TOP, 0); // shed C
        self.emit(Opcode::POP_TOP, 0); // shed V
        self.emit_jump(Opcode::JUMP, fail);
        self.fix_label(done);
        Ok(())
    }

    /// `P as name` in a value position ([X, V] on stack): check P against
    /// a copy, bind on success. Success and failure both exit [X].
    fn compile_as_value_match(
        &mut self,
        inner: &Pattern,
        name: &str,
        fail: usize,
    ) -> Result<(), String> {
        self.emit(Opcode::DUP_TOP, 0);
        let inner_fail = self.new_label();
        let done = self.new_label();
        self.compile_value_check(inner, inner_fail)?;
        self.emit_match_store(name);
        self.emit_jump(Opcode::JUMP, done);
        self.fix_label(inner_fail);
        self.emit(Opcode::POP_TOP, 0);
        self.emit_jump(Opcode::JUMP, fail);
        self.fix_label(done);
        Ok(())
    }

    pub(crate) fn compile_match_stmt(
        &mut self,
        subject: &Expr,
        cases: &[MatchCase],
    ) -> Result<(), String> {
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
                        // Value miss AND guard-false both mean "next
                        // case" — the old guard_false label fell through
                        // to the body on a value miss (PATMA-162/165).
                        self.emit_jump(Opcode::POP_JUMP_IF_FALSE, next_case);
                        let guard = case.guard.as_ref().unwrap();
                        self.compile_expr(guard)?;
                        self.emit_jump(Opcode::POP_JUMP_IF_FALSE, next_case);
                    } else {
                        self.emit_jump(Opcode::POP_JUMP_IF_FALSE, next_case);
                    }
                }
                Pattern::MatchAs {
                    pattern: Some(inner),
                    name: Some(n),
                } => {
                    // `0 | 1 | 2 as z` (as binds the whole or): check the
                    // inner pattern first, then bind. Entry [s].
                    self.emit(Opcode::DUP_TOP, 0); // [s, s]
                    self.compile_value_check(inner, next_case)?; // [s]
                    self.emit(Opcode::DUP_TOP, 0); // [s, s]
                    self.emit_match_store(n); // [s]
                    if let Some(guard) = &case.guard {
                        self.compile_expr(guard)?;
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
                        _ => ConstValue::String(s.clone().into()),
                    }) as u32;
                    self.emit(Opcode::LOAD_CONST, const_idx);
                    self.emit(Opcode::COMPARE_OP, 8); // IS
                    if let Some(guard) = &case.guard {
                        // Same next_case discipline as MatchValue above.
                        self.emit_jump(Opcode::POP_JUMP_IF_FALSE, next_case);
                        self.compile_expr(guard)?;
                        self.emit_jump(Opcode::POP_JUMP_IF_FALSE, next_case);
                    } else {
                        self.emit_jump(Opcode::POP_JUMP_IF_FALSE, next_case);
                    }
                }
                Pattern::MatchSequence(patterns) => {
                    // MatchSequence: check length and match elements
                    let star_index = patterns
                        .iter()
                        .position(|p| matches!(p, Pattern::MatchStar { .. }));
                    if let Some(si) = star_index {
                        // Non-raising type gate first (no DUP before it:
                        // a dup'd copy would leak on the failure path).
                        // `len()` on an int/None (or SUBSCR on a set)
                        // raised instead of failing the case — e.g.
                        // `match 0: case [a]:` must simply not match
                        // (PATMA-081..086, PATMA-053).
                        self.emit(Opcode::MATCH_SEQUENCE, 0);
                        self.emit_jump(Opcode::POP_JUMP_IF_FALSE, next_case);
                        // A sole wildcard star (`case [*_]` — parsed as
                        // `MatchStar { name: Some("_") }`, since `_` is a
                        // name token) matches any sequence with NO
                        // unpacking at all — CPython never iterates (a
                        // NAMED `[*r]` does unpack and may raise, e.g.
                        // Sequence with `__getitem__ = None`: PATMA-185).
                        // UNPACK_EX would call __getitem__.
                        let sole_wildcard_star = patterns.len() == 1
                            && match &patterns[0] {
                                Pattern::MatchStar { name } => {
                                    matches!(name.as_deref(), None | Some("_"))
                                }
                                Pattern::MatchAs {
                                    pattern: None,
                                    name: None,
                                } => true,
                                _ => false,
                            };
                        if sole_wildcard_star {
                            // Gate already left [S]: nothing to unpack,
                            // nothing to bind — fall through to guard/body.
                        } else {
                            self.emit(Opcode::DUP_TOP, 0);
                            // Get length of subject
                            let len_name_idx = self.get_name_index("len") as u32;
                            self.emit(Opcode::LOAD_GLOBAL, len_name_idx);
                            self.emit(Opcode::SWAP, 1);
                            self.emit(Opcode::CALL, 1);
                            // With MatchStar: require len(subject) >= patterns.len() - 1
                            let min_len = patterns.len() - 1;
                            let length_const =
                                self.get_const_index(ConstValue::Int(min_len.to_string())) as u32;
                            self.emit(Opcode::LOAD_CONST, length_const);
                            self.emit(Opcode::COMPARE_OP, 3); // >= (this VM's numbering: 3=>=, 4=>, 5=>!= — NOT CPython's 5=>=)
                            self.emit_jump(Opcode::POP_JUMP_IF_FALSE, next_case);
                            // Use UNPACK_EX to extract elements into natural order on the stack.
                            // Bit 16 (see the VM's UNPACK_EX): set when the star
                            // target is a wildcard (`*_`, parsed as
                            // `MatchStar { name: Some("_") }`) — its list is
                            // dropped, so a huge/unsized middle is never
                            // materialized (PATMA-192: 2**30 range).
                            let before = si;
                            let after = patterns.len() - si - 1;
                            let mut unpack_arg = ((before << 8) | after) as u32;
                            if matches!(
                                &patterns[si],
                                Pattern::MatchStar { name }
                                    if matches!(name.as_deref(), None | Some("_"))
                            ) || matches!(
                                &patterns[si],
                                Pattern::MatchAs {
                                    pattern: None,
                                    name: None
                                }
                            ) {
                                unpack_arg |= 1 << 16;
                            }
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
                                    Pattern::MatchAs {
                                        pattern: Some(inner),
                                        name: Some(n),
                                    } => {
                                        self.compile_as_value_match(inner, n, next_case)?;
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
                                            _ => ConstValue::String(s.clone().into()),
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
                        } // end else (non-trivial star shape: unpacked above)
                    } else {
                        // No MatchStar: exact length check + sequential
                        // extraction (shared with Or alternatives).
                        self.compile_sequence_exact_match(patterns, next_case)?;
                    }
                    if let Some(guard) = &case.guard {
                        self.compile_expr(guard)?;
                        self.emit_jump(Opcode::POP_JUMP_IF_FALSE, next_case);
                    }
                }
                Pattern::MatchMapping { keys, rest } => {
                    // Shared with Or alternatives (there fail = next
                    // alternative instead of next case).
                    self.compile_mapping_match(keys, rest, next_case)?;
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
                    // Flatten nested `(a | b) | c`: grouping parens now
                    // parse straight to MatchOr, so nesting is just list
                    // structure — alternatives are order-sensitive only.
                    let mut flat: Vec<&Pattern> = Vec::new();
                    flatten_or_patterns(subpatterns, &mut flat);
                    for pat in flat {
                        match pat {
                            Pattern::MatchAs {
                                pattern: None,
                                name: Some(n),
                            } => {
                                self.emit(Opcode::DUP_TOP, 0);
                                self.emit_match_store(n);
                                self.emit_jump(Opcode::JUMP, or_matched);
                            }
                            Pattern::MatchAs {
                                pattern: Some(inner),
                                name: Some(n),
                            } => {
                                // `0 as z`: the inner check consumes the
                                // dup'd copy (failure exits [o]); on
                                // success re-dup for the store. Uniform
                                // helper covers value/singleton/or/gates.
                                self.emit(Opcode::DUP_TOP, 0);
                                let try_next = self.new_label();
                                self.compile_value_check(inner, try_next)?;
                                // Inner check passed ([o]): re-dup the
                                // subject for the store.
                                self.emit(Opcode::DUP_TOP, 0);
                                self.emit_match_store(n);
                                self.emit_jump(Opcode::JUMP, or_matched);
                                self.fix_label(try_next);
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
                                    _ => ConstValue::String(s.clone().into()),
                                }) as u32;
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
                                // Same funnel as the plain (non-Or)
                                // MatchClass arm, but a miss falls through
                                // to the next alternative (alt_fail)
                                // instead of the next case.
                                self.compile_expr(cls)?; // [S, C]
                                let kwd_tuple_idx = self
                                    .get_const_index(ConstValue::Tuple(kwd_attrs.clone()))
                                    as u32;
                                self.emit(Opcode::LOAD_CONST, kwd_tuple_idx); // [S,C,kwd]
                                self.emit(Opcode::MATCH_CLASS_CHECK, patterns.len() as u32); // [S,C,bool]
                                let alt_fail = self.new_label();
                                self.emit_jump(Opcode::POP_JUMP_IF_FALSE, alt_fail); // [S,C]
                                for (i, sub) in patterns.iter().enumerate() {
                                    self.emit(Opcode::MATCH_CLASS_GETATTR, i as u32);
                                    self.compile_value_check(sub, alt_fail)?;
                                }
                                for (kwd_attr, kwd_pat) in kwd_attrs.iter().zip(kwd_patterns.iter())
                                {
                                    self.emit(Opcode::COPY, 1); // [S, C, S]
                                    let attr_idx = self.get_name_index(kwd_attr) as u32;
                                    self.emit(Opcode::LOAD_ATTR, attr_idx);
                                    self.compile_value_check(kwd_pat, alt_fail)?;
                                }
                                self.emit(Opcode::POP_TOP, 0); // [S]
                                self.emit_jump(Opcode::JUMP, or_matched);
                                self.fix_label(alt_fail);
                                self.emit(Opcode::POP_TOP, 0); // shed C -> [S]
                            }
                            Pattern::MatchMapping { keys, rest } => {
                                // Mapping alternative: match against a copy
                                // so failure leaves the original subject
                                // for the next alternative. The helper
                                // exits [.., X] on every path; drop the
                                // copy on both success and failure.
                                self.emit(Opcode::DUP_TOP, 0);
                                let alt_failed = self.new_label();
                                self.compile_mapping_match(keys, rest, alt_failed)?;
                                self.emit(Opcode::POP_TOP, 0);
                                self.emit_jump(Opcode::JUMP, or_matched);
                                self.fix_label(alt_failed);
                                self.emit(Opcode::POP_TOP, 0);
                            }
                            Pattern::MatchSequence(patterns)
                                if !patterns
                                    .iter()
                                    .any(|p| matches!(p, Pattern::MatchStar { .. })) =>
                            {
                                // Same shape as the mapping alternative
                                // above. Star-sequences can't reuse the
                                // exact matcher (UNPACK_EX leftovers), so
                                // they stay lenient non-match below.
                                self.emit(Opcode::DUP_TOP, 0);
                                let alt_failed = self.new_label();
                                self.compile_sequence_exact_match(patterns, alt_failed)?;
                                self.emit(Opcode::POP_TOP, 0);
                                self.emit_jump(Opcode::JUMP, or_matched);
                                self.fix_label(alt_failed);
                                self.emit(Opcode::POP_TOP, 0);
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
                    // Class pattern via the MATCH_CLASS_CHECK funnel
                    // (entry [S]): compile the class ONCE (a second
                    // evaluation per subpattern would repeat arbitrary
                    // side effects), build the kwd-name tuple, CHECK
                    // (not-a-class TypeError -> isinstance bool ->
                    // __match_args__ validation, CPython
                    // `_PyEval_MatchClass` order). CHECK exits
                    // [S, C, bool]; every miss path sheds C via
                    // sub_fail so next_case always sees [S].
                    self.compile_expr(cls)?; // [S, C]
                    let kwd_tuple_idx =
                        self.get_const_index(ConstValue::Tuple(kwd_attrs.clone())) as u32;
                    self.emit(Opcode::LOAD_CONST, kwd_tuple_idx); // [S, C, kwd]
                    self.emit(Opcode::MATCH_CLASS_CHECK, patterns.len() as u32); // [S,C,bool]
                    let sub_fail = self.new_label();
                    self.emit_jump(Opcode::POP_JUMP_IF_FALSE, sub_fail); // [S, C]

                    // Positional subpatterns, extracted via
                    // __match_args__ names (never by slot).
                    for (i, pat) in patterns.iter().enumerate() {
                        self.emit(Opcode::MATCH_CLASS_GETATTR, i as u32); // [S,C,attr]
                        self.compile_value_check(pat, sub_fail)?; // [S, C]
                    }

                    // Keyword subpatterns.
                    for (kwd_attr, kwd_pat) in kwd_attrs.iter().zip(kwd_patterns.iter()) {
                        self.emit(Opcode::COPY, 1); // [S, C, S]
                        let attr_idx = self.get_name_index(kwd_attr) as u32;
                        self.emit(Opcode::LOAD_ATTR, attr_idx); // [S, C, attr]
                        self.compile_value_check(kwd_pat, sub_fail)?; // [S, C]
                    }

                    self.emit(Opcode::POP_TOP, 0); // drop C -> [S]
                    if let Some(guard) = &case.guard {
                        self.compile_expr(guard)?;
                        self.emit_jump(Opcode::POP_JUMP_IF_FALSE, next_case);
                    }
                    // Miss trampoline: shed C so the shared next_case
                    // (and the tail's subject POP) always sees [S]. The
                    // success path jumps OVER it via body (falling
                    // through would pop the live subject).
                    let body = self.new_label();
                    self.emit_jump(Opcode::JUMP, body);
                    self.fix_label(sub_fail);
                    self.emit(Opcode::POP_TOP, 0);
                    self.emit_jump(Opcode::JUMP, next_case);
                    self.fix_label(body);
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

/// Flatten nested or-patterns (`(a | b) | c` from grouping parens) into a
/// single alternative list, preserving source order.
fn flatten_or_patterns<'a>(pats: &'a [Pattern], out: &mut Vec<&'a Pattern>) {
    for p in pats {
        if let Pattern::MatchOr(inner) = p {
            flatten_or_patterns(inner, out);
        } else {
            out.push(p);
        }
    }
}
