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

        let mut saved_indices: Vec<(String, u32)> = Vec::new();
        if !target_names.is_empty() {
            for name in &target_names {
                let tmp_name = format!("_comp_save_{}_{}", comp_id, name);
                let idx = self.add_varname(&tmp_name) as u32;
                saved_indices.push((name.clone(), idx));
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
            }
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

        if !saved_indices.is_empty() {
            let none_idx = self.get_const_index(ConstValue::None) as u32;
            for (name, save_idx) in &saved_indices {
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

        let mut saved_indices: Vec<(String, u32)> = Vec::new();
        if !target_names.is_empty() {
            for name in &target_names {
                let tmp_name = format!("_comp_save_{}_{}", comp_id, name);
                let idx = self.add_varname(&tmp_name) as u32;
                saved_indices.push((name.clone(), idx));
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
            }
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

        if !saved_indices.is_empty() {
            let none_idx = self.get_const_index(ConstValue::None) as u32;
            for (name, save_idx) in &saved_indices {
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

        Ok(())
    }
}
