use crate::ast::*;
use crate::bytecode::*;
use std::collections::HashSet;
use super::Compiler;

impl Compiler {
    pub(crate) fn compile_comprehension(
        &mut self,
        elt: &Expr,
        generators: &[Comprehension],
        is_set: bool,
    ) -> Result<(), String> {
        if generators.is_empty() {
            return Err("Comprehension must have at least one generator".to_string());
        }

        let mut target_names: Vec<String> = Vec::new();
        for gen in generators {
            let mut names = HashSet::new();
            Self::collect_assign_target_names(&gen.target, &mut names);
            for n in names {
                target_names.push(n);
            }
        }
        target_names.sort();
        target_names.dedup();

        let comp_id = self.comprehension_depth;
        self.comprehension_depth += 1;

        let saved_indices = self.compile_comprehension_save(comp_id, &target_names);

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

        self.compile_comprehension_restore(&saved_indices);

        Ok(())
    }

    pub(crate) fn compile_dict_comprehension(
        &mut self,
        key: &Expr,
        value: &Expr,
        generators: &[Comprehension],
    ) -> Result<(), String> {
        if generators.is_empty() {
            return Err("Comprehension must have at least one generator".to_string());
        }

        let mut target_names: Vec<String> = Vec::new();
        for gen in generators {
            let mut names = HashSet::new();
            Self::collect_assign_target_names(&gen.target, &mut names);
            for n in names {
                target_names.push(n);
            }
        }
        target_names.sort();
        target_names.dedup();

        let comp_id = self.comprehension_depth;
        self.comprehension_depth += 1;

        let saved_indices = self.compile_comprehension_save(comp_id, &target_names);

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

        self.compile_comprehension_restore(&saved_indices);

        Ok(())
    }

    /// Save-phase strategy for one comprehension target name, decided once
    /// at compile time based on how `name` resolves in the enclosing scope.
    fn compile_comprehension_save(
        &mut self,
        comp_id: usize,
        target_names: &[String],
    ) -> Vec<(String, u32, CompSaveKind)> {
        let mut saved_indices = Vec::new();
        for name in target_names {
            let tmp_name = format!("_comp_save_{}_{}", comp_id, name);
            let idx = self.add_varname(&tmp_name) as u32;
            if self.is_plain_local_name(name) {
                // `had_idx` records (as a plain bool, no exceptions
                // involved at restore time) whether `name` had a prior
                // binding, so restore can tell "never existed" apart from
                // "existed and was `None`" precisely.
                let had_name = format!("_comp_had_{}_{}", comp_id, name);
                let had_idx = self.add_varname(&had_name) as u32;
                let handler = self.new_label();
                let end = self.new_label();
                self.emit_jump(Opcode::SETUP_FINALLY, handler);
                self.emit_load_name(name);
                self.emit(Opcode::STORE_FAST, idx);
                let true_idx = self.get_const_index(ConstValue::Bool(true)) as u32;
                self.emit(Opcode::LOAD_CONST, true_idx);
                self.emit(Opcode::STORE_FAST, had_idx);
                self.emit(Opcode::POP_BLOCK, 0);
                self.emit_jump(Opcode::JUMP, end);
                self.fix_label(handler);
                self.emit(Opcode::PUSH_EXC_INFO, 0);
                let false_idx = self.get_const_index(ConstValue::Bool(false)) as u32;
                self.emit(Opcode::LOAD_CONST, false_idx);
                self.emit(Opcode::STORE_FAST, had_idx);
                self.emit(Opcode::POP_EXCEPT, 1);
                self.fix_label(end);
                saved_indices.push((name.clone(), idx, CompSaveKind::Plain { had_idx }));
            } else {
                let handler = self.new_label();
                let end = self.new_label();
                self.emit_jump(Opcode::SETUP_FINALLY, handler);
                self.emit_load_name(name);
                self.emit(Opcode::STORE_FAST, idx);
                self.emit(Opcode::POP_BLOCK, 0);
                self.emit_jump(Opcode::JUMP, end);
                self.fix_label(handler);
                self.emit(Opcode::PUSH_EXC_INFO, 0);
                let none_idx = self.get_const_index(ConstValue::None) as u32;
                self.emit(Opcode::LOAD_CONST, none_idx);
                self.emit(Opcode::STORE_FAST, idx);
                self.emit(Opcode::POP_EXCEPT, 1);
                self.fix_label(end);
                saved_indices.push((name.clone(), idx, CompSaveKind::Legacy));
            }
        }
        saved_indices
    }

    fn compile_comprehension_restore(&mut self, saved_indices: &[(String, u32, CompSaveKind)]) {
        for (name, save_idx, kind) in saved_indices {
            match *kind {
                CompSaveKind::Plain { had_idx } => {
                    // Was `name` bound before this comprehension ran? If
                    // not, unbind it again instead of leaving it holding the
                    // comprehension's last iterated value forever
                    // (previously left `name` invisible to `dir()` yet
                    // still readable via plain `name`, pinning whatever
                    // object it last held — root cause of test_weakset's
                    // intersection/union undercounting after gc.collect(),
                    // traced to `[x for x in items]` never releasing the
                    // final `x`). No exceptions here (unlike the save phase)
                    // — this runs on every comprehension execution, not
                    // just the rare pre-existing-name case.
                    let no_restore = self.new_label();
                    let end = self.new_label();
                    self.emit(Opcode::LOAD_FAST, had_idx);
                    self.emit_jump(Opcode::POP_JUMP_IF_FALSE, no_restore);
                    self.emit(Opcode::LOAD_FAST, *save_idx);
                    self.emit_store_name(name);
                    self.emit_jump(Opcode::JUMP, end);
                    self.fix_label(no_restore);
                    self.emit_delete_name(name);
                    self.fix_label(end);
                }
                CompSaveKind::Legacy => {
                    let none_idx = self.get_const_index(ConstValue::None) as u32;
                    let skip_restore = self.new_label();
                    self.emit(Opcode::LOAD_FAST, *save_idx);
                    self.emit(Opcode::LOAD_CONST, none_idx);
                    self.emit(Opcode::IS_OP, 1);
                    self.emit_jump(Opcode::POP_JUMP_IF_FALSE, skip_restore);
                    self.emit(Opcode::LOAD_FAST, *save_idx);
                    self.emit_store_name(name);
                    self.fix_label(skip_restore);
                }
            }
        }
    }
}

/// See `Compiler::compile_comprehension_save`/`compile_comprehension_restore`.
enum CompSaveKind {
    /// A plain function-local: restore knows for certain whether `name`
    /// existed before (via `had_idx`), so it can properly unbind it when it
    /// didn't rather than leaving a stale value dangling.
    Plain { had_idx: u32 },
    /// A module/class-body name, or a cellvar/freevar shared with a nested
    /// closure defined inside the comprehension body: fall back to the
    /// original sentinel-based restore, which never unbinds — only restores
    /// when the saved value isn't the `None` sentinel. Unbinding/rebinding
    /// these broke closures capturing the comprehension's loop variable
    /// (`test_listcomps.py`'s `test_lambda_in_iter` et al.) — real CPython
    /// avoids this entirely by giving comprehensions their own scope; this
    /// codebase inlines them into the enclosing scope instead (a known gap,
    /// see `GAP_ANALYSIS.md`).
    Legacy,
}
