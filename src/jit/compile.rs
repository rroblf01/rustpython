use crate::bytecode::{CodeObject, Opcode};
use crate::object::PyObjectRef;
use cranelift::prelude::*;
use cranelift::codegen::ir::{StackSlot, StackSlotData, StackSlotKind, FuncRef};
use cranelift_module::{Linkage, Module};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::cell::RefCell;
use super::compiler::JitCompiler;


pub(crate) struct FuncRefs {
    pub add: FuncRef,
    pub sub: FuncRef,
    pub mul: FuncRef,
    pub div: FuncRef,
    pub floor_div: FuncRef,
    pub r#mod: FuncRef,
    pub pow: FuncRef,
    pub lshift: FuncRef,
    pub rshift: FuncRef,
    pub bit_and: FuncRef,
    pub bit_or: FuncRef,
    pub bit_xor: FuncRef,
    pub inplace_binop: FuncRef,
    pub getitem: FuncRef,
    pub cmp: FuncRef,
    pub truthy: FuncRef,
    pub neg: FuncRef,
    pub not: FuncRef,
    pub build_list: FuncRef,
    pub build_tuple: FuncRef,
    pub list_append: FuncRef,
    pub contains: FuncRef,
    pub get_iter: FuncRef,
    pub call: FuncRef,
    pub call_kw: FuncRef,
    pub load_attr: FuncRef,
    pub for_iter: FuncRef,
    pub build_map: FuncRef,
    pub store_attr: FuncRef,
    pub unpack_sequence: FuncRef,
    pub load_name: FuncRef,
    pub build_set: FuncRef,
    pub build_string: FuncRef,
    pub build_slice: FuncRef,
    pub store_subscr: FuncRef,
    pub is_op: FuncRef,
    pub invert: FuncRef,
    pub import_name: FuncRef,
    pub import_from: FuncRef,
    pub unpack_ex: FuncRef,
    pub setup_with: FuncRef,
    pub with_exit: FuncRef,
    pub make_function: FuncRef,
}

pub(crate) struct CompileState<'a> {
    pub locals_slot: StackSlot,
    pub consts_ptr: Value,
    pub result_ptr: Value,
    pub code: &'a CodeObject,
    pub func_refs: FuncRefs,
    pub block_of: &'a HashMap<usize, Block>,
    pub instr_to_block: &'a HashMap<usize, Block>,
    pub blocks_entered: &'a mut HashSet<Block>,
    pub terminated: &'a mut bool,
    pub i: usize,
}

impl JitCompiler {
    pub fn compile(
        &mut self,
        code: &CodeObject,
    ) -> Option<extern "C" fn(*const PyObjectRef, usize, *const PyObjectRef, *mut PyObjectRef)>
    {
        if !Self::is_enabled() {
            return None;
        }
        // Relaxed restrictions: allow *args, **kwargs, keyword-only params,
        // and default values. The JIT compiles the function body, not the
        // calling convention — these params are passed via fast_locals or
        // the stack, and the JIT handles them correctly.
        if code.instructions.is_empty() || code.instructions.len() > 200 {
            return None;
        }
        // Only compile functions with loops (back edges) — JIT shines for loops
        let has_loop = code
            .instructions
            .iter()
            .any(|i| matches!(i.op, Opcode::JUMP_BACKWARD | Opcode::FOR_ITER));
        if !has_loop {
            return None;
        }

        // Check all opcodes are supported
        let supported: &[Opcode] = &[
            Opcode::LOAD_FAST,
            Opcode::LOAD_CONST,
            Opcode::BINARY_OP,
            Opcode::RETURN_VALUE,
            Opcode::STORE_FAST,
            Opcode::DUP_TOP,
            Opcode::POP_TOP,
            Opcode::COMPARE_OP,
            Opcode::POP_JUMP_IF_FALSE,
            Opcode::JUMP_BACKWARD,
            Opcode::JUMP_FORWARD,
            Opcode::JUMP,
            Opcode::LOAD_GLOBAL,
            Opcode::UNARY_NEGATIVE,
            Opcode::UNARY_NOT,
            Opcode::BUILD_LIST,
            Opcode::BUILD_TUPLE,
            Opcode::LIST_APPEND,
            Opcode::CONTAINS_OP,
            Opcode::CALL,
            Opcode::PUSH_NULL,
            Opcode::LOAD_ATTR,
            Opcode::GET_ITER,
            Opcode::FOR_ITER,
            Opcode::BUILD_MAP,
            Opcode::STORE_ATTR,
            Opcode::UNPACK_SEQUENCE,
            Opcode::LOAD_NAME,
            Opcode::POP_JUMP_IF_TRUE,
            Opcode::POP_JUMP_IF_NONE,
            Opcode::POP_JUMP_IF_NOT_NONE,
            Opcode::COPY,
            Opcode::SWAP,
            Opcode::BUILD_SET,
            Opcode::BUILD_SLICE,
            Opcode::BUILD_STRING,
            Opcode::STORE_SUBSCR,
            Opcode::IS_OP,
            Opcode::UNARY_INVERT,
            Opcode::IMPORT_NAME,
            Opcode::IMPORT_FROM,
            Opcode::UNPACK_EX,
            Opcode::SETUP_WITH,
            Opcode::WITH_EXIT,
            Opcode::MAKE_FUNCTION,
            Opcode::END_FOR,
            Opcode::MAP_ADD,
            Opcode::SETUP_FINALLY,
        ];
        for instr in &code.instructions {
            if !supported.contains(&instr.op) {
                // Gated behind RPY_DEBUG_JIT: this fires for most real
                // functions (generators, closures, try/except...) and used
                // to print unconditionally, polluting every test's stderr.
                if std::env::var("RPY_DEBUG_JIT").is_ok() || std::env::var("RPY_DEBUG_JIT_ALL").is_ok() {
                    eprintln!("JIT: unsupported opcode {:?} in '{}'", instr.op, code.name);
                }
                return None;
            }
            // BINARY_OP's arg encodes: 0..=12 a plain operator (see
            // compile_expr's Expr::BinOp), 13 = BINARY_SUBSCR (see
            // compiler.rs's `self.emit(Opcode::BINARY_OP, 13)` call sites),
            // and 100..=112 the in-place variant of operator `arg - 100`
            // (AugAssign's codegen — the only emitter of that range). Any
            // other value isn't reachable from this compiler's own codegen.
            if instr.op == Opcode::BINARY_OP {
                let arg = instr.arg;
                // 12 (plain matmul) is deliberately excluded: no native
                // numeric type implements `@`, so there's no fast-path
                // func_ref for it below (only the in-place variant, 112,
                // is handled — via jit_py_inplace_binop's own dunder
                // dispatch — since AugAssign is the only realistic emitter
                // of `@` in JIT-eligible hot loops).
                let valid = arg <= 11 || arg == 13 || (100..=112).contains(&arg);
                if !valid {
                    eprintln!("JIT: unsupported BINARY_OP arg {} in '{}'", arg, code.name);
                    return None;
                }
            }
        }

        let _consts = Self::precompute_with_names(code);

        let mut sig =
            cranelift::codegen::ir::Signature::new(cranelift::codegen::isa::CallConv::SystemV);
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));

        let mut ctx = cranelift::codegen::Context::new();
        ctx.func.signature = sig.clone();
        let func = self
            .module
            .declare_function("jit_fn", Linkage::Local, &sig)
            .ok()?;

        let add_func_ref = self
            .module
            .declare_func_in_func(self.add_func, &mut ctx.func);
        let sub_func_ref = self
            .module
            .declare_func_in_func(self.sub_func, &mut ctx.func);
        let mul_func_ref = self
            .module
            .declare_func_in_func(self.mul_func, &mut ctx.func);
        let div_func_ref = self
            .module
            .declare_func_in_func(self.div_func, &mut ctx.func);
        let floor_div_func_ref = self
            .module
            .declare_func_in_func(self.floor_div_func, &mut ctx.func);
        let mod_func_ref = self
            .module
            .declare_func_in_func(self.mod_func, &mut ctx.func);
        let pow_func_ref = self
            .module
            .declare_func_in_func(self.pow_func, &mut ctx.func);
        let lshift_func_ref = self
            .module
            .declare_func_in_func(self.lshift_func, &mut ctx.func);
        let rshift_func_ref = self
            .module
            .declare_func_in_func(self.rshift_func, &mut ctx.func);
        let bit_and_func_ref = self
            .module
            .declare_func_in_func(self.bit_and_func, &mut ctx.func);
        let bit_or_func_ref = self
            .module
            .declare_func_in_func(self.bit_or_func, &mut ctx.func);
        let bit_xor_func_ref = self
            .module
            .declare_func_in_func(self.bit_xor_func, &mut ctx.func);
        let inplace_binop_func_ref = self
            .module
            .declare_func_in_func(self.inplace_binop_func, &mut ctx.func);
        let getitem_func_ref = self
            .module
            .declare_func_in_func(self.getitem_func, &mut ctx.func);
        let cmp_func_ref = self
            .module
            .declare_func_in_func(self.cmp_func, &mut ctx.func);
        let truthy_func_ref = self
            .module
            .declare_func_in_func(self.truthy_func, &mut ctx.func);
        let neg_func_ref = self
            .module
            .declare_func_in_func(self.neg_func, &mut ctx.func);
        let not_func_ref = self
            .module
            .declare_func_in_func(self.not_func, &mut ctx.func);
        let build_list_func_ref = self
            .module
            .declare_func_in_func(self.build_list_func, &mut ctx.func);
        let build_tuple_func_ref = self
            .module
            .declare_func_in_func(self.build_tuple_func, &mut ctx.func);
        let list_append_func_ref = self
            .module
            .declare_func_in_func(self.list_append_func, &mut ctx.func);
        let contains_func_ref = self
            .module
            .declare_func_in_func(self.contains_func, &mut ctx.func);
        let get_iter_func_ref = self
            .module
            .declare_func_in_func(self.get_iter_func, &mut ctx.func);
        let call_func_ref = self
            .module
            .declare_func_in_func(self.call_func, &mut ctx.func);
        let call_kw_func_ref = self
            .module
            .declare_func_in_func(self.call_kw_func, &mut ctx.func);
        let load_attr_func_ref = self
            .module
            .declare_func_in_func(self.load_attr_func, &mut ctx.func);
        let for_iter_func_ref = self
            .module
            .declare_func_in_func(self.for_iter_func, &mut ctx.func);
        let build_map_func_ref = self
            .module
            .declare_func_in_func(self.build_map_func, &mut ctx.func);
        let store_attr_func_ref = self
            .module
            .declare_func_in_func(self.store_attr_func, &mut ctx.func);
        let unpack_sequence_func_ref = self
            .module
            .declare_func_in_func(self.unpack_sequence_func, &mut ctx.func);
        let load_name_func_ref = self
            .module
            .declare_func_in_func(self.load_name_func, &mut ctx.func);
        let build_set_func_ref = self
            .module
            .declare_func_in_func(self.build_set_func, &mut ctx.func);
        let build_string_func_ref = self
            .module
            .declare_func_in_func(self.build_string_func, &mut ctx.func);
        let build_slice_func_ref = self
            .module
            .declare_func_in_func(self.build_slice_func, &mut ctx.func);
        let store_subscr_func_ref = self
            .module
            .declare_func_in_func(self.store_subscr_func, &mut ctx.func);
        let is_op_func_ref = self
            .module
            .declare_func_in_func(self.is_op_func, &mut ctx.func);
        let invert_func_ref = self
            .module
            .declare_func_in_func(self.invert_func, &mut ctx.func);
        let import_name_func_ref = self
            .module
            .declare_func_in_func(self.import_name_func, &mut ctx.func);
        let import_from_func_ref = self
            .module
            .declare_func_in_func(self.import_from_func, &mut ctx.func);
        let unpack_ex_func_ref = self
            .module
            .declare_func_in_func(self.unpack_ex_func, &mut ctx.func);
        let setup_with_func_ref = self
            .module
            .declare_func_in_func(self.setup_with_func, &mut ctx.func);
        let with_exit_func_ref = self
            .module
            .declare_func_in_func(self.with_exit_func, &mut ctx.func);
        let make_function_func_ref = self
            .module
            .declare_func_in_func(self.make_function_func, &mut ctx.func);

        // Pre-scan for branch targets
        let mut targets: HashSet<usize> = HashSet::new();
        targets.insert(0);
        for (i, instr) in code.instructions.iter().enumerate() {
            match instr.op {
                Opcode::POP_JUMP_IF_FALSE => {
                    // Both the target and the fallthrough are potential block starts
                    if instr.arg as usize != i + 1 {
                        targets.insert(instr.arg as usize);
                    }
                    targets.insert(i + 1);
                }
                Opcode::JUMP_BACKWARD => {
                    let target = i.wrapping_sub(instr.arg as usize);
                    targets.insert(target);
                    targets.insert(i + 1);
                }
                Opcode::FOR_ITER => {
                    let target = instr.arg as usize;
                    targets.insert(target);
                    targets.insert(i + 1);
                }
                Opcode::POP_JUMP_IF_TRUE
                | Opcode::POP_JUMP_IF_NONE
                | Opcode::POP_JUMP_IF_NOT_NONE => {
                    if instr.arg as usize != i + 1 {
                        targets.insert(instr.arg as usize);
                    }
                    targets.insert(i + 1);
                }
                Opcode::JUMP_FORWARD | Opcode::JUMP => {
                    // Unconditional jump — its target must be a block
                    // boundary too (the actual codegen below, at the
                    // `Opcode::JUMP_FORWARD | Opcode::JUMP` arm, looks it
                    // up via `block_of[&target]`; omitting it here left
                    // that lookup panicking with "no entry found for key"
                    // whenever a jump's target wasn't ALREADY a block
                    // boundary for some unrelated reason, e.g. an `if/else`
                    // whose `else` arm ends in a plain unconditional jump
                    // past it). No fallthrough entry needed here, unlike
                    // the conditional jumps above: an unconditional jump
                    // always terminates its block, so `i + 1` is dead code
                    // and not a real predecessor.
                    let target = i + instr.arg as usize;
                    targets.insert(target);
                }
                _ => {}
            }
        }

        let mut builder = FunctionBuilder::new(&mut ctx.func, &mut self.builder_context);

        // Create blocks for each target
        let mut block_of: HashMap<usize, Block> = HashMap::new();
        let mut sorted_targets: Vec<usize> = targets.into_iter().collect();
        sorted_targets.sort();
        for &t in &sorted_targets {
            let b = builder.create_block();
            block_of.insert(t, b);
        }

        // Map each instruction to its containing block
        let mut instr_to_block: HashMap<usize, Block> = HashMap::new();
        let mut current_block_idx = 0;
        for i in 0..code.instructions.len() {
            if block_of.contains_key(&i) {
                current_block_idx = i;
            }
            instr_to_block.insert(i, block_of[&current_block_idx]);
        }

        // Track which blocks have been entered
        let mut blocks_entered: HashSet<Block> = HashSet::new();

        // Process entry block
        let entry_block = block_of[&0];
        builder.append_block_params_for_function_params(entry_block);
        builder.switch_to_block(entry_block);
        blocks_entered.insert(entry_block);

        let args_ptr = builder.block_params(entry_block)[0];
        let _nargs = builder.block_params(entry_block)[1];
        let consts_ptr = builder.block_params(entry_block)[2];
        let result_ptr = builder.block_params(entry_block)[3];

        // Allocate locals array on stack
        let locals_size = (code.nlocals.max(1) * 24) as u32;
        let locals_slot = builder.create_sized_stack_slot(StackSlotData::new(
            StackSlotKind::ExplicitSlot,
            locals_size,
            0,
        ));

        // Copy args to locals
        for i in 0..code.arg_count.min(code.nlocals) {
            let src = builder.ins().iadd_imm(args_ptr, (i * 24) as i64);
            let dst = builder
                .ins()
                .stack_addr(types::I64, locals_slot, (i * 24) as i32);
            let zero = builder.ins().iconst(types::I64, 0);
            let lo =
                builder
                    .ins()
                    .load(types::I64, cranelift::codegen::ir::MemFlags::new(), src, 0);
            let hi =
                builder
                    .ins()
                    .load(types::I64, cranelift::codegen::ir::MemFlags::new(), src, 8);
            builder
                .ins()
                .store(cranelift::codegen::ir::MemFlags::new(), lo, dst, 0);
            builder
                .ins()
                .store(cranelift::codegen::ir::MemFlags::new(), hi, dst, 8);
            builder
                .ins()
                .store(cranelift::codegen::ir::MemFlags::new(), zero, dst, 16);
        }

        // Evaluation stack
        let mut eval_stack: Vec<[Value; 3]> = Vec::new();

        // Pre-allocate temp stack slots for BINARY_OP, CALL, STORE_ATTR, etc.
        let tmp_slot1 = builder.create_sized_stack_slot(StackSlotData::new(
            cranelift::codegen::ir::StackSlotKind::ExplicitSlot,
            24,
            0,
        ));
        let tmp_slot2 = builder.create_sized_stack_slot(StackSlotData::new(
            cranelift::codegen::ir::StackSlotKind::ExplicitSlot,
            24,
            0,
        ));
        let tmp_out = builder.create_sized_stack_slot(StackSlotData::new(
            cranelift::codegen::ir::StackSlotKind::ExplicitSlot,
            24,
            0,
        ));

        if cfg!(feature = "profile") {
            eprintln!(
                "JIT_DEBUG: starting codegen for {} instructions",
                code.instructions.len()
            );
        }
        // Track whether current block has been terminated (needs jump for fallthrough)
        let mut terminated = false;
        // Generate code for each instruction
        for i in 0..code.instructions.len() {
            let block = instr_to_block[&i];

            // Switch to the correct block if not already there
            if builder.current_block() != Some(block) {
                if !terminated {
                    builder.ins().jump(block, &[]);
                }
                builder.switch_to_block(block);
                blocks_entered.insert(block);
                terminated = false;
            } else if terminated {
                // Same block as the previous instruction, but that block
                // was ALREADY given a terminator (RETURN_VALUE/JUMP/
                // JUMP_BACKWARD/brif) — this instruction is genuinely
                // unreachable dead code within it. The most common real
                // trigger: an explicit `return x` inside a loop, followed
                // by the compiler's own always-appended (whether reachable
                // or not) implicit `LOAD_CONST None; RETURN_VALUE`
                // fallback at the very end of every function body — real
                // CPython bytecode has the exact same trailing
                // return-None fallback, it's just genuinely unreachable
                // there too whenever an earlier explicit return already
                // fired. Cranelift disallows adding MORE instructions to
                // an already-terminated block ("cannot add an instruction
                // to a block already filled") — confirmed via a minimal
                // repro (any JIT-eligible loop in a function with an
                // explicit `return` crashed the whole process). This
                // dead code must be skipped entirely, not emitted.
                continue;
            }

            let instr = &code.instructions[i];
            let func_refs = FuncRefs {
                add: add_func_ref,
                sub: sub_func_ref,
                mul: mul_func_ref,
                div: div_func_ref,
                floor_div: floor_div_func_ref,
                r#mod: mod_func_ref,
                pow: pow_func_ref,
                lshift: lshift_func_ref,
                rshift: rshift_func_ref,
                bit_and: bit_and_func_ref,
                bit_or: bit_or_func_ref,
                bit_xor: bit_xor_func_ref,
                inplace_binop: inplace_binop_func_ref,
                getitem: getitem_func_ref,
                cmp: cmp_func_ref,
                truthy: truthy_func_ref,
                neg: neg_func_ref,
                not: not_func_ref,
                build_list: build_list_func_ref,
                build_tuple: build_tuple_func_ref,
                list_append: list_append_func_ref,
                contains: contains_func_ref,
                get_iter: get_iter_func_ref,
                call: call_func_ref,
                call_kw: call_kw_func_ref,
                load_attr: load_attr_func_ref,
                for_iter: for_iter_func_ref,
                build_map: build_map_func_ref,
                store_attr: store_attr_func_ref,
                unpack_sequence: unpack_sequence_func_ref,
                load_name: load_name_func_ref,
                build_set: build_set_func_ref,
                build_string: build_string_func_ref,
                build_slice: build_slice_func_ref,
                store_subscr: store_subscr_func_ref,
                is_op: is_op_func_ref,
                invert: invert_func_ref,
                import_name: import_name_func_ref,
                import_from: import_from_func_ref,
                unpack_ex: unpack_ex_func_ref,
                setup_with: setup_with_func_ref,
                with_exit: with_exit_func_ref,
                make_function: make_function_func_ref,
            };
            {
                let func_refs = FuncRefs {
                    add: add_func_ref,
                    sub: sub_func_ref,
                    mul: mul_func_ref,
                    div: div_func_ref,
                    floor_div: floor_div_func_ref,
                    r#mod: mod_func_ref,
                    pow: pow_func_ref,
                    lshift: lshift_func_ref,
                    rshift: rshift_func_ref,
                    bit_and: bit_and_func_ref,
                    bit_or: bit_or_func_ref,
                    bit_xor: bit_xor_func_ref,
                    inplace_binop: inplace_binop_func_ref,
                    getitem: getitem_func_ref,
                    cmp: cmp_func_ref,
                    truthy: truthy_func_ref,
                    neg: neg_func_ref,
                    not: not_func_ref,
                    build_list: build_list_func_ref,
                    build_tuple: build_tuple_func_ref,
                    list_append: list_append_func_ref,
                    contains: contains_func_ref,
                    get_iter: get_iter_func_ref,
                    call: call_func_ref,
                    call_kw: call_kw_func_ref,
                    load_attr: load_attr_func_ref,
                    for_iter: for_iter_func_ref,
                    build_map: build_map_func_ref,
                    store_attr: store_attr_func_ref,
                    unpack_sequence: unpack_sequence_func_ref,
                    load_name: load_name_func_ref,
                    build_set: build_set_func_ref,
                    build_string: build_string_func_ref,
                    build_slice: build_slice_func_ref,
                    store_subscr: store_subscr_func_ref,
                    is_op: is_op_func_ref,
                    invert: invert_func_ref,
                    import_name: import_name_func_ref,
                    import_from: import_from_func_ref,
                    unpack_ex: unpack_ex_func_ref,
                    setup_with: setup_with_func_ref,
                    with_exit: with_exit_func_ref,
                    make_function: make_function_func_ref,
                };
                let mut ctx = CompileState {
                    locals_slot,
                    consts_ptr,
                    result_ptr,
                    code,
                    func_refs,
                    block_of: &block_of,
                    instr_to_block: &instr_to_block,
                    blocks_entered: &mut blocks_entered,
                    terminated: &mut terminated,
                    i,
                };
                if crate::jit::emit::emit_part1(&mut builder, &mut eval_stack, &mut ctx, instr) {
                } else if crate::jit::emit2::emit_part2(&mut builder, &mut eval_stack, &mut ctx, instr) {
                } else {
                    eprintln!("JIT: codegen unsupported {:?} at instr {}", instr.op, i);
                    return None;
                }
            }
        }
        builder.seal_all_blocks();
        builder.finalize();
        match self.module.define_function(func, &mut ctx) {
            Ok(_) => {}
            Err(_) => {
                return None;
            }
        }
        self.module.finalize_definitions().ok()?;
        let code_ptr = self.module.get_finalized_function(func);
        // SAFETY: `code_ptr` is the address of the machine code just emitted
        // by this function's own IR-building above, which always declares
        // `func` with exactly this signature (4 params: 2 `*const PyObjectRef`
        // + usize + `*mut PyObjectRef` — see the `Signature` built earlier in
        // `compile()`), so the transmute target matches the actual ABI.
        Some(unsafe { std::mem::transmute(code_ptr) })
    }
}
