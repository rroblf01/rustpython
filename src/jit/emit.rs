use crate::bytecode::{CodeObject, Opcode};
use crate::object::PyObjectRef;
use cranelift::prelude::*;
use std::collections::{HashMap, HashSet};
use super::compile::CompileState;

pub(crate) fn emit_part1(builder: &mut FunctionBuilder, eval_stack: &mut Vec<[Value; 3]>, ctx: &mut CompileState, instr: &crate::bytecode::Instr) -> bool {
    match instr.op {

                Opcode::LOAD_FAST => {
                    let idx = instr.arg as i32;
                    let memflags = cranelift::codegen::ir::MemFlags::new();
                    let src = builder.ins().stack_addr(types::I64, ctx.locals_slot, idx * 24);
                    let lo = builder.ins().load(types::I64, memflags, src, 0);
                    let hi = builder.ins().load(types::I64, memflags, src, 8);
                    let hi2 = builder.ins().load(types::I64, memflags, src, 16);
                    eval_stack.push([lo, hi, hi2]);
                }
                Opcode::LOAD_CONST => {
                    let idx = instr.arg as i32;
                    let memflags = cranelift::codegen::ir::MemFlags::new();
                    let src = builder.ins().iadd_imm(ctx.consts_ptr, (idx * 24) as i64);
                    let lo = builder.ins().load(types::I64, memflags, src, 0);
                    let hi = builder.ins().load(types::I64, memflags, src, 8);
                    let hi2 = builder.ins().load(types::I64, memflags, src, 16);
                    eval_stack.push([lo, hi, hi2]);
                }
                Opcode::LOAD_GLOBAL => {
                    let name_idx = instr.arg as i32;
                    let consts_count = ctx.code.consts.len() as i64;
                    let idx = name_idx as i64 + consts_count;
                    let memflags = cranelift::codegen::ir::MemFlags::new();
                    let src = builder.ins().iadd_imm(ctx.consts_ptr, (idx * 24) as i64);
                    let lo = builder.ins().load(types::I64, memflags, src, 0);
                    let hi = builder.ins().load(types::I64, memflags, src, 8);
                    let hi2 = builder.ins().load(types::I64, memflags, src, 16);
                    eval_stack.push([lo, hi, hi2]);
                }
                Opcode::BINARY_OP => {
                    let b = eval_stack.pop().unwrap();
                    let a = eval_stack.pop().unwrap();
                    let memflags = cranelift::codegen::ir::MemFlags::new();
                    let tmp_a = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot,
                        24,
                        0,
                    ));
                    let tmp_b = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot,
                        24,
                        0,
                    ));
                    let tmp_out = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot,
                        24,
                        0,
                    ));
                    let a_addr = builder.ins().stack_addr(types::I64, tmp_a, 0);
                    let b_addr = builder.ins().stack_addr(types::I64, tmp_b, 0);
                    let out_addr = builder.ins().stack_addr(types::I64, tmp_out, 0);
                    builder.ins().store(memflags, a[0], a_addr, 0);
                    builder.ins().store(memflags, a[1], a_addr, 8);
                    builder.ins().store(memflags, b[0], b_addr, 0);
                    builder.ins().store(memflags, b[1], b_addr, 8);
                    // arg encoding (must match compiler.rs's `bin_op` tables
                    // exactly): 0=add 1=sub 2=mul 3=div 4=floordiv 5=mod
                    // 6=pow 7=lshift 8=rshift 9=bitor 10=bitxor 11=bitand,
                    // 13=SUBSCR (no in-place form), 100+n=in-place variant
                    // of operator n. `12` (matmul) has no JIT fast path (no
                    // native numeric type implements `@`) and is excluded
                    // from the `supported` gate's arg range on purpose.
                    if instr.arg == 13 {
                        builder
                            .ins()
                            .call(ctx.func_refs.getitem, &[a_addr, b_addr, out_addr]);
                    } else if instr.arg >= 100 {
                        let op_val = builder.ins().iconst(types::I64, (instr.arg - 100) as i64);
                        builder
                            .ins()
                            .call(ctx.func_refs.inplace_binop, &[a_addr, b_addr, op_val, out_addr]);
                    } else {
                        let func_ref = match instr.arg {
                            0 => ctx.func_refs.add,
                            1 => ctx.func_refs.sub,
                            2 => ctx.func_refs.mul,
                            3 => ctx.func_refs.div,
                            4 => ctx.func_refs.floor_div,
                            5 => ctx.func_refs.r#mod,
                            6 => ctx.func_refs.pow,
                            7 => ctx.func_refs.lshift,
                            8 => ctx.func_refs.rshift,
                            9 => ctx.func_refs.bit_or,
                            10 => ctx.func_refs.bit_xor,
                            11 => ctx.func_refs.bit_and,
                            _ => unreachable!(),
                        };
                        builder.ins().call(func_ref, &[a_addr, b_addr, out_addr]);
                    }
                    let res_lo = builder.ins().load(types::I64, memflags, out_addr, 0);
                    let res_hi = builder.ins().load(types::I64, memflags, out_addr, 8);
                    let res_mid = builder.ins().load(types::I64, memflags, out_addr, 16);
                    eval_stack.push([res_lo, res_hi, res_mid]);
                }
                Opcode::COMPARE_OP => {
                    let b = eval_stack.pop().unwrap();
                    let a = eval_stack.pop().unwrap();
                    let memflags = cranelift::codegen::ir::MemFlags::new();

                    let tmp_a = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot,
                        24,
                        0,
                    ));
                    let tmp_b = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot,
                        24,
                        0,
                    ));
                    let tmp_out = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot,
                        24,
                        0,
                    ));

                    let a_addr = builder.ins().stack_addr(types::I64, tmp_a, 0);
                    let b_addr = builder.ins().stack_addr(types::I64, tmp_b, 0);
                    let out_addr = builder.ins().stack_addr(types::I64, tmp_out, 0);

                    builder.ins().store(memflags, a[0], a_addr, 0);
                    builder.ins().store(memflags, a[1], a_addr, 8);
                    builder.ins().store(memflags, b[0], b_addr, 0);
                    builder.ins().store(memflags, b[1], b_addr, 8);

                    let op_val = builder.ins().iconst(types::I64, instr.arg as i64);
                    builder
                        .ins()
                        .call(ctx.func_refs.cmp, &[a_addr, b_addr, op_val, out_addr]);
                    let res_lo = builder.ins().load(types::I64, memflags, out_addr, 0);
                    let res_hi = builder.ins().load(types::I64, memflags, out_addr, 8);
                    let res_mid = builder.ins().load(types::I64, memflags, out_addr, 16);
                    eval_stack.push([res_lo, res_hi, res_mid]);
                }
                Opcode::STORE_FAST => {
                    let idx = instr.arg as i32;
                    let memflags = cranelift::codegen::ir::MemFlags::new();
                    let val = eval_stack.pop().unwrap();
                    let zero = builder.ins().iconst(types::I64, 0);
                    let dst = builder.ins().stack_addr(types::I64, ctx.locals_slot, idx * 24);
                    builder.ins().store(memflags, val[0], dst, 0);
                    builder.ins().store(memflags, val[1], dst, 8);
                    builder.ins().store(memflags, val[2], dst, 16);
                }
                Opcode::PUSH_NULL => {
                    let zero = builder.ins().iconst(types::I64, 0);
                    eval_stack.push([zero, zero, zero]);
                }
                Opcode::DUP_TOP => {
                    let val = eval_stack.last().unwrap();
                    eval_stack.push([val[0], val[1], val[2]]);
                }
                Opcode::POP_TOP | Opcode::END_FOR => {
                    // END_FOR just pops the for-loop iterator on natural
                    // exhaustion (see compiler.rs's `LoopInfo::is_for` doc
                    // comment) — an unconditional stack pop, same as
                    // POP_TOP, with no dependent codegen of its own.
                    eval_stack.pop();
                }
                Opcode::POP_JUMP_IF_FALSE => {
                    let val = eval_stack.pop().unwrap();
                    let memflags = cranelift::codegen::ir::MemFlags::new();

                    let tmp_val = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot,
                        24,
                        0,
                    ));

                    let val_addr = builder.ins().stack_addr(types::I64, tmp_val, 0);

                    builder.ins().store(memflags, val[0], val_addr, 0);
                    builder.ins().store(memflags, val[1], val_addr, 8);
                    builder.ins().store(memflags, val[2], val_addr, 16);
                    let truthy_inst = builder.ins().call(ctx.func_refs.truthy, &[val_addr]);
                    let truthy = builder.inst_results(truthy_inst)[0];
                    let zero = builder.ins().iconst(types::I64, 0);
                    let cmp = builder.ins().icmp(IntCC::Equal, truthy, zero);

                    let target = instr.arg as usize;
                    let target_block = ctx.block_of[&target];
                    let next_block = ctx.block_of[&(ctx.i + 1)];

                    builder.ins().brif(cmp, target_block, &[], next_block, &[]);
                    *ctx.terminated = true;
                }
                Opcode::POP_JUMP_IF_TRUE
                | Opcode::POP_JUMP_IF_NONE
                | Opcode::POP_JUMP_IF_NOT_NONE => {
                    let val = eval_stack.pop().unwrap();
                    let memflags = cranelift::codegen::ir::MemFlags::new();

                    let tmp_val = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot,
                        24,
                        0,
                    ));

                    let val_addr = builder.ins().stack_addr(types::I64, tmp_val, 0);

                    builder.ins().store(memflags, val[0], val_addr, 0);
                    builder.ins().store(memflags, val[1], val_addr, 8);
                    builder.ins().store(memflags, val[2], val_addr, 16);
                    let truthy_inst = builder.ins().call(ctx.func_refs.truthy, &[val_addr]);
                    let truthy = builder.inst_results(truthy_inst)[0];
                    let zero = builder.ins().iconst(types::I64, 0);
                    let cmp = builder.ins().icmp(IntCC::Equal, truthy, zero);

                    let target = instr.arg as usize;
                    let target_block = ctx.block_of[&target];
                    let next_block = ctx.block_of[&(ctx.i + 1)];

                    builder.ins().brif(cmp, target_block, &[], next_block, &[]);
                    *ctx.terminated = true;
                }
                Opcode::JUMP_BACKWARD => {
                    let target = ctx.i.wrapping_sub(instr.arg as usize);
                    let target_block = ctx.block_of[&target];
                    builder.ins().jump(target_block, &[]);
                    *ctx.terminated = true;
                }
                Opcode::JUMP_FORWARD | Opcode::JUMP => {
                    let target = ctx.i + instr.arg as usize;
                    let target_block = ctx.block_of[&target];
                    builder.ins().jump(target_block, &[]);
                    *ctx.terminated = true;
                }
                Opcode::UNARY_NEGATIVE => {
                    let val = eval_stack.pop().unwrap();
                    let memflags = cranelift::codegen::ir::MemFlags::new();
                    let tmp_val = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot,
                        24,
                        0,
                    ));
                    let tmp_out = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot,
                        24,
                        0,
                    ));
                    let val_addr = builder.ins().stack_addr(types::I64, tmp_val, 0);
                    let out_addr = builder.ins().stack_addr(types::I64, tmp_out, 0);
                    builder.ins().store(memflags, val[0], val_addr, 0);
                    builder.ins().store(memflags, val[1], val_addr, 8);
                    builder.ins().store(memflags, val[2], val_addr, 16);
                    builder.ins().call(ctx.func_refs.neg, &[val_addr, out_addr]);
                    let res_lo = builder.ins().load(types::I64, memflags, out_addr, 0);
                    let res_hi = builder.ins().load(types::I64, memflags, out_addr, 8);
                    let res_mid = builder.ins().load(types::I64, memflags, out_addr, 16);
                    eval_stack.push([res_lo, res_hi, res_mid]);
                }
                Opcode::UNARY_NOT => {
                    let val = eval_stack.pop().unwrap();
                    let memflags = cranelift::codegen::ir::MemFlags::new();
                    let tmp_val = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot,
                        24,
                        0,
                    ));
                    let tmp_out = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot,
                        24,
                        0,
                    ));
                    let val_addr = builder.ins().stack_addr(types::I64, tmp_val, 0);
                    let out_addr = builder.ins().stack_addr(types::I64, tmp_out, 0);
                    builder.ins().store(memflags, val[0], val_addr, 0);
                    builder.ins().store(memflags, val[1], val_addr, 8);
                    builder.ins().store(memflags, val[2], val_addr, 16);
                    builder.ins().call(ctx.func_refs.not, &[val_addr, out_addr]);
                    let res_lo = builder.ins().load(types::I64, memflags, out_addr, 0);
                    let res_hi = builder.ins().load(types::I64, memflags, out_addr, 8);
                    let res_mid = builder.ins().load(types::I64, memflags, out_addr, 16);
                    eval_stack.push([res_lo, res_hi, res_mid]);
                }
                Opcode::BUILD_LIST => {
                    let n = instr.arg as usize;
                    let memflags = cranelift::codegen::ir::MemFlags::new();
                    let mut items: Vec<[Value; 3]> = Vec::with_capacity(n);
                    for _ in 0..n {
                        items.push(eval_stack.pop().unwrap());
                    }
                    items.reverse();
                    let array_size = ((n * 24).max(16)) as u32;
                    let array_slot = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot,
                        array_size,
                        0,
                    ));
                    let tmp_out = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot,
                        24,
                        0,
                    ));
                    let array_addr = builder.ins().stack_addr(types::I64, array_slot, 0);
                    let out_addr = builder.ins().stack_addr(types::I64, tmp_out, 0);
                    for (i, item) in items.iter().enumerate() {
                        let offset = (i * 24) as i32;
                        let item_addr = builder.ins().iadd_imm(array_addr, offset as i64);
                        builder.ins().store(memflags, item[0], item_addr, 0);
                        builder.ins().store(memflags, item[1], item_addr, 8);
                        builder.ins().store(memflags, item[2], item_addr, 16);
                    }
                    let n_val = builder.ins().iconst(types::I64, n as i64);
                    builder
                        .ins()
                        .call(ctx.func_refs.build_list, &[n_val, array_addr, out_addr]);
                    let res_lo = builder.ins().load(types::I64, memflags, out_addr, 0);
                    let res_hi = builder.ins().load(types::I64, memflags, out_addr, 8);
                    let res_mid = builder.ins().load(types::I64, memflags, out_addr, 16);
                    eval_stack.push([res_lo, res_hi, res_mid]);
                }
                Opcode::BUILD_TUPLE => {
                    let n = instr.arg as usize;
                    let memflags = cranelift::codegen::ir::MemFlags::new();
                    let mut items: Vec<[Value; 3]> = Vec::with_capacity(n);
                    for _ in 0..n {
                        items.push(eval_stack.pop().unwrap());
                    }
                    items.reverse();
                    let array_size = ((n * 24).max(16)) as u32;
                    let array_slot = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot,
                        array_size,
                        0,
                    ));
                    let tmp_out = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot,
                        24,
                        0,
                    ));
                    let array_addr = builder.ins().stack_addr(types::I64, array_slot, 0);
                    let out_addr = builder.ins().stack_addr(types::I64, tmp_out, 0);
                    for (i, item) in items.iter().enumerate() {
                        let offset = (i * 24) as i32;
                        let item_addr = builder.ins().iadd_imm(array_addr, offset as i64);
                        builder.ins().store(memflags, item[0], item_addr, 0);
                        builder.ins().store(memflags, item[1], item_addr, 8);
                        builder.ins().store(memflags, item[2], item_addr, 16);
                    }
                    let n_val = builder.ins().iconst(types::I64, n as i64);
                    builder
                        .ins()
                        .call(ctx.func_refs.build_tuple, &[n_val, array_addr, out_addr]);
                    let res_lo = builder.ins().load(types::I64, memflags, out_addr, 0);
                    let res_hi = builder.ins().load(types::I64, memflags, out_addr, 8);
                    let res_mid = builder.ins().load(types::I64, memflags, out_addr, 16);
                    eval_stack.push([res_lo, res_hi, res_mid]);
                }
                Opcode::LIST_APPEND => {
                    let val = eval_stack.pop().unwrap();
                    let lst = eval_stack.pop().unwrap();
                    let memflags = cranelift::codegen::ir::MemFlags::new();
                    let tmp_lst = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot,
                        24,
                        0,
                    ));
                    let tmp_val = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot,
                        24,
                        0,
                    ));
                    let tmp_out = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot,
                        24,
                        0,
                    ));
                    let lst_addr = builder.ins().stack_addr(types::I64, tmp_lst, 0);
                    let val_addr = builder.ins().stack_addr(types::I64, tmp_val, 0);
                    let out_addr = builder.ins().stack_addr(types::I64, tmp_out, 0);
                    builder.ins().store(memflags, lst[0], lst_addr, 0);
                    builder.ins().store(memflags, lst[1], lst_addr, 8);
                    builder.ins().store(memflags, val[0], val_addr, 0);
                    builder.ins().store(memflags, val[1], val_addr, 8);
                    builder.ins().store(memflags, val[2], val_addr, 16);
                    builder
                        .ins()
                        .call(ctx.func_refs.list_append, &[lst_addr, val_addr, out_addr]);
                }
                Opcode::CONTAINS_OP => {
                    let b = eval_stack.pop().unwrap();
                    let a = eval_stack.pop().unwrap();
                    let memflags = cranelift::codegen::ir::MemFlags::new();
                    let tmp_a = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot,
                        24,
                        0,
                    ));
                    let tmp_b = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot,
                        24,
                        0,
                    ));
                    let tmp_out = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot,
                        24,
                        0,
                    ));
                    let a_addr = builder.ins().stack_addr(types::I64, tmp_a, 0);
                    let b_addr = builder.ins().stack_addr(types::I64, tmp_b, 0);
                    let out_addr = builder.ins().stack_addr(types::I64, tmp_out, 0);
                    builder.ins().store(memflags, a[0], a_addr, 0);
                    builder.ins().store(memflags, a[1], a_addr, 8);
                    builder.ins().store(memflags, b[0], b_addr, 0);
                    builder.ins().store(memflags, b[1], b_addr, 8);
                    builder
                        .ins()
                        .call(ctx.func_refs.contains, &[a_addr, b_addr, out_addr]);
                    let res_lo = builder.ins().load(types::I64, memflags, out_addr, 0);
                    let res_hi = builder.ins().load(types::I64, memflags, out_addr, 8);
                    let res_mid = builder.ins().load(types::I64, memflags, out_addr, 16);
                    eval_stack.push([res_lo, res_hi, res_mid]);
                }
                Opcode::GET_ITER => {
                    let val = eval_stack.pop().unwrap();
                    let memflags = cranelift::codegen::ir::MemFlags::new();
                    let tmp_val = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot,
                        24,
                        0,
                    ));
                    let tmp_out = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot,
                        24,
                        0,
                    ));
                    let val_addr = builder.ins().stack_addr(types::I64, tmp_val, 0);
                    let out_addr = builder.ins().stack_addr(types::I64, tmp_out, 0);
                    builder.ins().store(memflags, val[0], val_addr, 0);
                    builder.ins().store(memflags, val[1], val_addr, 8);
                    builder.ins().store(memflags, val[2], val_addr, 16);
                    builder.ins().call(ctx.func_refs.get_iter, &[val_addr, out_addr]);
                    let res_lo = builder.ins().load(types::I64, memflags, out_addr, 0);
                    let res_hi = builder.ins().load(types::I64, memflags, out_addr, 8);
                    let res_mid = builder.ins().load(types::I64, memflags, out_addr, 16);
                    eval_stack.push([res_lo, res_hi, res_mid]);
                }
                Opcode::CALL => {
                    // `arg` packs BOTH counts (matching vm.rs's own
                    // `Opcode::CALL` handler exactly): `npos = arg & 0xFF`
                    // positional args, `nkw = (arg >> 8) & 0xFF` keyword
                    // (name, value) pairs following them on the stack.
                    // Previously read `instr.arg` directly AS `nargs` —
                    // correct only when `nkw == 0` (arg fits in the low
                    // byte unchanged); any call with at least one keyword
                    // argument packs a nonzero `nkw` into the upper bits,
                    // which got misread as thousands of extra positional
                    // args to pop (real trigger: `cache_from_source(source,
                    // optimization=opt)` compiling to `CALL arg=257` —
                    // `1 | (1 << 8)` — inside a JIT-eligible loop, popping
                    // 257 stack slots instead of 2 and panicking with a
                    // `None`-value `unwrap()` on the exhausted `eval_stack`).
                    let npos = instr.arg as usize & 0xFF;
                    let nkw = (instr.arg as usize >> 8) & 0xFF;
                    let total = npos + 2 * nkw;
                    let memflags = cranelift::codegen::ir::MemFlags::new();
                    let mut args: Vec<[Value; 3]> = Vec::with_capacity(total);
                    for _ in 0..total {
                        args.push(eval_stack.pop().unwrap());
                    }
                    let func = eval_stack.pop().unwrap();
                    args.reverse();
                    let array_size = ((total * 24).max(16)) as u32;
                    let tmp_func = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot,
                        24,
                        0,
                    ));
                    let array_slot = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot,
                        array_size,
                        0,
                    ));
                    let tmp_out = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot,
                        24,
                        0,
                    ));
                    let func_addr = builder.ins().stack_addr(types::I64, tmp_func, 0);
                    let array_addr = builder.ins().stack_addr(types::I64, array_slot, 0);
                    let out_addr = builder.ins().stack_addr(types::I64, tmp_out, 0);
                    builder.ins().store(memflags, func[0], func_addr, 0);
                    builder.ins().store(memflags, func[1], func_addr, 8);
                    for (i, item) in args.iter().enumerate() {
                        let offset = (i * 24) as i32;
                        let item_addr = builder.ins().iadd_imm(array_addr, offset as i64);
                        builder.ins().store(memflags, item[0], item_addr, 0);
                        builder.ins().store(memflags, item[1], item_addr, 8);
                        builder.ins().store(memflags, item[2], item_addr, 16);
                    }
                    if nkw == 0 {
                        let nargs_val = builder.ins().iconst(types::I64, npos as i64);
                        builder
                            .ins()
                            .call(ctx.func_refs.call, &[func_addr, nargs_val, array_addr, out_addr]);
                    } else {
                        let npos_val = builder.ins().iconst(types::I64, npos as i64);
                        let nkw_val = builder.ins().iconst(types::I64, nkw as i64);
                        builder.ins().call(
                            ctx.func_refs.call_kw,
                            &[func_addr, npos_val, nkw_val, array_addr, out_addr],
                        );
                    }
                    let res_lo = builder.ins().load(types::I64, memflags, out_addr, 0);
                    let res_hi = builder.ins().load(types::I64, memflags, out_addr, 8);
                    let res_mid = builder.ins().load(types::I64, memflags, out_addr, 16);
                    eval_stack.push([res_lo, res_hi, res_mid]);
                }
                
        _ => return false,
    }
    true
}
