use crate::bytecode::{CodeObject, Opcode};
use crate::object::PyObjectRef;
use cranelift::prelude::*;
use std::collections::{HashMap, HashSet};
use super::compile::CompileState;

pub(crate) fn emit_part2(builder: &mut FunctionBuilder, eval_stack: &mut Vec<[Value; 3]>, ctx: &mut CompileState, instr: &crate::bytecode::Instr) -> bool {
    match instr.op {
Opcode::LOAD_ATTR => {
                    let name_idx = instr.arg as i64;
                    let val = eval_stack.pop().unwrap();
                    let memflags = cranelift::codegen::ir::MemFlags::new();
                    let names_offset = (ctx.code.consts.len() + ctx.code.names.len()) as i64;
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
                    let names_ptr = builder.ins().iadd_imm(ctx.consts_ptr, names_offset * 24);
                    let name_idx_val = builder.ins().iconst(types::I64, name_idx);
                    builder.ins().call(
                        ctx.func_refs.load_attr,
                        &[val_addr, names_ptr, name_idx_val, out_addr],
                    );
                    let res_lo = builder.ins().load(types::I64, memflags, out_addr, 0);
                    let res_hi = builder.ins().load(types::I64, memflags, out_addr, 8);
                    let res_mid = builder.ins().load(types::I64, memflags, out_addr, 16);
                    eval_stack.push([res_lo, res_hi, res_mid]);
                }
                Opcode::MAKE_FUNCTION => {
                    // MAKE_FUNCTION: pop closure, defaults, ctx.code, name, create function
                    let has_closure = (instr.arg & 0x100) != 0;
                    let n_defaults = (instr.arg & 0xFF) as usize;
                    let n_kwdefaults = ((instr.arg >> 9) & 0xFF) as usize;
                    let n_items = n_defaults + n_kwdefaults + 1 + if has_closure { 1 } else { 0 };
                    let memflags = cranelift::codegen::ir::MemFlags::new();
                    let mut items: Vec<[Value; 3]> = Vec::with_capacity(n_items);
                    for _ in 0..n_items {
                        items.push(eval_stack.pop().unwrap());
                    }
                    items.reverse();
                    let array_size = ((n_items * 24).max(16)) as u32;
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
                    let arg_val = builder.ins().iconst(types::I64, instr.arg as i64);
                    builder
                        .ins()
                        .call(ctx.func_refs.make_function, &[array_addr, arg_val, out_addr]);
                    let res_lo = builder.ins().load(types::I64, memflags, out_addr, 0);
                    let res_hi = builder.ins().load(types::I64, memflags, out_addr, 8);
                    let res_mid = builder.ins().load(types::I64, memflags, out_addr, 16);
                    eval_stack.push([res_lo, res_hi, res_mid]);
                }
                Opcode::RETURN_VALUE => {
                    let val = eval_stack.pop().unwrap();
                    let memflags = cranelift::codegen::ir::MemFlags::new();
                    let zero = builder.ins().iconst(types::I64, 0);
                    builder.ins().store(memflags, val[0], ctx.result_ptr, 0);
                    builder.ins().store(memflags, val[1], ctx.result_ptr, 8);
                    builder.ins().store(memflags, val[2], ctx.result_ptr, 16);
                    builder.ins().store(memflags, val[2], ctx.result_ptr, 16);
                    builder.ins().return_(&[]);
                    *ctx.terminated = true;
                }
                Opcode::FOR_ITER => {
                    // PEEK, not pop: vm.rs's own FOR_ITER does
                    // `self.frames[fi].peek(0)` — the iterator stays on the
                    // stack for every subsequent pass (this same
                    // instruction runs again next iteration via
                    // JUMP_BACKWARD) and is only ever popped once, by
                    // END_FOR, on natural exhaustion. Popping it here
                    // desynced this codegen's simulated `eval_stack` from
                    // the real bytecode stack shape by one slot for the
                    // rest of the loop body — never caught before because
                    // END_FOR wasn't JIT-supported until now, so no
                    // real-world for-loop ever reached this path.
                    let iter = *eval_stack.last().unwrap();
                    let memflags = cranelift::codegen::ir::MemFlags::new();
                    let tmp_iter = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot,
                        24,
                        0,
                    ));
                    let tmp_out = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot,
                        24,
                        0,
                    ));
                    let iter_addr = builder.ins().stack_addr(types::I64, tmp_iter, 0);
                    let out_addr = builder.ins().stack_addr(types::I64, tmp_out, 0);
                    builder.ins().store(memflags, iter[0], iter_addr, 0);
                    builder.ins().store(memflags, iter[1], iter_addr, 8);
                    let iter_result = builder
                        .ins()
                        .call(ctx.func_refs.for_iter, &[iter_addr, out_addr]);
                    let status = builder.inst_results(iter_result)[0];
                    let zero = builder.ins().iconst(types::I64, 0);
                    let has_value = builder.ins().icmp(IntCC::Equal, status, zero);
                    let target = instr.arg as usize;
                    let target_block = ctx.block_of[&target];
                    let next_block = ctx.block_of[&(ctx.i + 1)];
                    builder
                        .ins()
                        .brif(has_value, next_block, &[], target_block, &[]);
                    // `brif` terminates the CURRENT block — the loads below
                    // belong to the "has a value" continuation, which is a
                    // SEPARATE block (`next_block`) that must be switched
                    // into before appending anything else. Emitting them
                    // without switching first tried to add instructions to
                    // the block `brif` had just filled, panicking with
                    // "cannot add an instruction to a block already
                    // filled" — dormant until END_FOR made real for-loops
                    // reach the JIT at all (see FOR_ITER's peek/pop note
                    // above, same root cause class).
                    builder.switch_to_block(next_block);
                    ctx.blocks_entered.insert(next_block);
                    let res_lo = builder.ins().load(types::I64, memflags, out_addr, 0);
                    let res_hi = builder.ins().load(types::I64, memflags, out_addr, 8);
                    let res_mid = builder.ins().load(types::I64, memflags, out_addr, 16);
                    eval_stack.push([res_lo, res_hi, res_mid]);
                }
                Opcode::BUILD_MAP => {
                    let n = instr.arg as usize;
                    let memflags = cranelift::codegen::ir::MemFlags::new();
                    let mut items: Vec<[Value; 3]> = Vec::with_capacity(n * 2);
                    for _ in 0..n * 2 {
                        items.push(eval_stack.pop().unwrap());
                    }
                    items.reverse();
                    let array_size = ((n * 2 * 24).max(16)) as u32;
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
                        .call(ctx.func_refs.build_map, &[n_val, array_addr, out_addr]);
                    let res_lo = builder.ins().load(types::I64, memflags, out_addr, 0);
                    let res_hi = builder.ins().load(types::I64, memflags, out_addr, 8);
                    let res_mid = builder.ins().load(types::I64, memflags, out_addr, 16);
                    eval_stack.push([res_lo, res_hi, res_mid]);
                }
                Opcode::STORE_ATTR => {
                    let name_idx = instr.arg as i64;
                    let val = eval_stack.pop().unwrap();
                    let obj = eval_stack.pop().unwrap();
                    let memflags = cranelift::codegen::ir::MemFlags::new();
                    let names_offset = (ctx.code.consts.len() + ctx.code.names.len()) as i64;
                    let tmp_obj = builder.create_sized_stack_slot(StackSlotData::new(
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
                    let obj_addr = builder.ins().stack_addr(types::I64, tmp_obj, 0);
                    let val_addr = builder.ins().stack_addr(types::I64, tmp_val, 0);
                    let out_addr = builder.ins().stack_addr(types::I64, tmp_out, 0);
                    builder.ins().store(memflags, obj[0], obj_addr, 0);
                    builder.ins().store(memflags, obj[1], obj_addr, 8);
                    builder.ins().store(memflags, val[0], val_addr, 0);
                    builder.ins().store(memflags, val[1], val_addr, 8);
                    builder.ins().store(memflags, val[2], val_addr, 16);
                    let names_ptr = builder.ins().iadd_imm(ctx.consts_ptr, names_offset * 24);
                    let name_idx_val = builder.ins().iconst(types::I64, name_idx);
                    builder.ins().call(
                        ctx.func_refs.store_attr,
                        &[obj_addr, names_ptr, name_idx_val, val_addr, out_addr],
                    );
                    let res_lo = builder.ins().load(types::I64, memflags, out_addr, 0);
                    let res_hi = builder.ins().load(types::I64, memflags, out_addr, 8);
                    let res_mid = builder.ins().load(types::I64, memflags, out_addr, 16);
                    eval_stack.push([res_lo, res_hi, res_mid]);
                }
                Opcode::UNPACK_SEQUENCE => {
                    let n = instr.arg as usize;
                    let seq = eval_stack.pop().unwrap();
                    let memflags = cranelift::codegen::ir::MemFlags::new();
                    let tmp_seq = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot,
                        24,
                        0,
                    ));
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
                    let seq_addr = builder.ins().stack_addr(types::I64, tmp_seq, 0);
                    let array_addr = builder.ins().stack_addr(types::I64, array_slot, 0);
                    let out_addr = builder.ins().stack_addr(types::I64, tmp_out, 0);
                    builder.ins().store(memflags, seq[0], seq_addr, 0);
                    builder.ins().store(memflags, seq[1], seq_addr, 8);
                    let n_val = builder.ins().iconst(types::I64, n as i64);
                    builder.ins().call(
                        ctx.func_refs.unpack_sequence,
                        &[seq_addr, n_val, array_addr, out_addr],
                    );
                    // Push unpacked items onto stack in order
                    for i in 0..n {
                        let offset = (i * 24) as i32;
                        let item_addr = builder.ins().iadd_imm(array_addr, offset as i64);
                        let ilo = builder.ins().load(types::I64, memflags, item_addr, 0);
                        let ihi = builder.ins().load(types::I64, memflags, item_addr, 8);
                        let imid = builder.ins().load(types::I64, memflags, item_addr, 16);
                        eval_stack.push([ilo, ihi, imid]);
                    }
                }
                Opcode::LOAD_NAME => {
                    let name_idx = instr.arg as i64;
                    let names_offset = (ctx.code.consts.len() + ctx.code.names.len()) as i64;
                    let memflags = cranelift::codegen::ir::MemFlags::new();
                    let tmp_locals = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot,
                        24,
                        0,
                    ));
                    let tmp_globals = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot,
                        24,
                        0,
                    ));
                    let tmp_out = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot,
                        24,
                        0,
                    ));
                    // For simplicity, use the ctx.result_ptr as the "globals" reference
                    // In a real JIT we'd need the actual globals dict
                    // Store locals array as a dict-like proxy
                    let locals_addr = builder.ins().stack_addr(types::I64, tmp_locals, 0);
                    let globals_addr = builder.ins().stack_addr(types::I64, tmp_globals, 0);
                    let out_addr = builder.ins().stack_addr(types::I64, tmp_out, 0);
                    // Use ctx.consts_ptr as globals proxy (precomputed_with_globals stores globals after consts)
                    // For simplicity, write the ctx.consts_ptr into both locals and globals slots
                    builder.ins().store(memflags, ctx.consts_ptr, locals_addr, 0);
                    builder.ins().store(memflags, ctx.consts_ptr, locals_addr, 8);
                    builder.ins().store(memflags, ctx.consts_ptr, globals_addr, 0);
                    builder.ins().store(memflags, ctx.consts_ptr, globals_addr, 8);
                    let names_ptr = builder.ins().iadd_imm(ctx.consts_ptr, names_offset * 24);
                    let name_idx_val = builder.ins().iconst(types::I64, name_idx);
                    builder.ins().call(
                        ctx.func_refs.load_name,
                        &[names_ptr, name_idx_val, locals_addr, globals_addr, out_addr],
                    );
                    let res_lo = builder.ins().load(types::I64, memflags, out_addr, 0);
                    let res_hi = builder.ins().load(types::I64, memflags, out_addr, 8);
                    let res_mid = builder.ins().load(types::I64, memflags, out_addr, 16);
                    eval_stack.push([res_lo, res_hi, res_mid]);
                }
                Opcode::COPY => {
                    // Mirrors vm.rs's own graceful fallback: when `depth`
                    // reaches or exceeds the stack depth (e.g. `a = b = 1`'s
                    // COPY(1) called with only the just-pushed constant on
                    // the stack — see `Stmt::Assign`'s multi-target
                    // codegen), vm.rs treats it as a plain DUP_TOP instead
                    // of indexing out of bounds. Indexing unconditionally
                    // here underflowed (`eval_stack.len() - 1 - depth`)
                    // and panicked with "attempt to subtract with
                    // overflow" — dormant until BINARY_OP's in-place range
                    // (100+) started letting loop-containing functions
                    // that also happen to chain-assign (`inner = outer =
                    // 1`) reach the JIT at all.
                    let depth = instr.arg as usize;
                    let val = if depth >= eval_stack.len() {
                        *eval_stack.last().unwrap()
                    } else {
                        eval_stack[eval_stack.len() - 1 - depth]
                    };
                    eval_stack.push(val);
                }
                Opcode::SWAP => {
                    // Mirror vm.rs's own bounds guard (`if ctx.i > 0 && ctx.i <
                    // len`, silently a no-op otherwise) rather than
                    // indexing unconditionally.
                    let len = eval_stack.len();
                    let i = instr.arg as usize;
                    if i > 0 && i < len {
                        eval_stack.swap(len - 1, len - 1 - i);
                    }
                }
                Opcode::POP_JUMP_IF_TRUE => {
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
                    let cmp = builder.ins().icmp(IntCC::NotEqual, truthy, zero);
                    let target = instr.arg as usize;
                    let target_block = ctx.block_of[&target];
                    let next_block = ctx.block_of[&(ctx.i + 1)];
                    builder.ins().brif(cmp, target_block, &[], next_block, &[]);
                }
                Opcode::POP_JUMP_IF_NONE => {
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
                    let is_none = builder.ins().icmp(IntCC::Equal, truthy, zero);
                    let target = instr.arg as usize;
                    let target_block = ctx.block_of[&target];
                    let next_block = ctx.block_of[&(ctx.i + 1)];
                    builder
                        .ins()
                        .brif(is_none, target_block, &[], next_block, &[]);
                }
                Opcode::POP_JUMP_IF_NOT_NONE => {
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
                    let is_not_none = builder.ins().icmp(IntCC::NotEqual, truthy, zero);
                    let target = instr.arg as usize;
                    let target_block = ctx.block_of[&target];
                    let next_block = ctx.block_of[&(ctx.i + 1)];
                    builder
                        .ins()
                        .brif(is_not_none, target_block, &[], next_block, &[]);
                }
                Opcode::BUILD_SET => {
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
                        .call(ctx.func_refs.build_set, &[n_val, array_addr, out_addr]);
                    let res_lo = builder.ins().load(types::I64, memflags, out_addr, 0);
                    let res_hi = builder.ins().load(types::I64, memflags, out_addr, 8);
                    let res_mid = builder.ins().load(types::I64, memflags, out_addr, 16);
                    eval_stack.push([res_lo, res_hi, res_mid]);
                }
                Opcode::BUILD_STRING => {
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
                        .call(ctx.func_refs.build_string, &[n_val, array_addr, out_addr]);
                    let res_lo = builder.ins().load(types::I64, memflags, out_addr, 0);
                    let res_hi = builder.ins().load(types::I64, memflags, out_addr, 8);
                    let res_mid = builder.ins().load(types::I64, memflags, out_addr, 16);
                    eval_stack.push([res_lo, res_hi, res_mid]);
                }
                Opcode::BUILD_SLICE => {
                    let nargs = instr.arg as usize;
                    if nargs < 2 || nargs > 3 {
                        return false;
                    }
                    let memflags = cranelift::codegen::ir::MemFlags::new();
                    let mut items: Vec<[Value; 3]> = Vec::with_capacity(3);
                    if nargs >= 3 {
                        items.push(eval_stack.pop().unwrap());
                    }
                    items.push(eval_stack.pop().unwrap());
                    items.push(eval_stack.pop().unwrap());
                    // items now has [start, stop, step_or_none] but we need [start, stop, step]
                    // items were pushed: start (3rd pop), stop (2nd pop), [step (1st pop if nargs==3)]
                    items.reverse();
                    let array_size = ((3 * 24).max(16)) as u32;
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
                    let n_val = builder.ins().iconst(types::I64, nargs as i64);
                    builder
                        .ins()
                        .call(ctx.func_refs.build_slice, &[n_val, array_addr, out_addr]);
                    let res_lo = builder.ins().load(types::I64, memflags, out_addr, 0);
                    let res_hi = builder.ins().load(types::I64, memflags, out_addr, 8);
                    let res_mid = builder.ins().load(types::I64, memflags, out_addr, 16);
                    eval_stack.push([res_lo, res_hi, res_mid]);
                }
                Opcode::STORE_SUBSCR => {
                    let val = eval_stack.pop().unwrap();
                    let idx = eval_stack.pop().unwrap();
                    let obj = eval_stack.pop().unwrap();
                    let memflags = cranelift::codegen::ir::MemFlags::new();
                    let tmp_obj = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot,
                        24,
                        0,
                    ));
                    let tmp_idx = builder.create_sized_stack_slot(StackSlotData::new(
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
                    let obj_addr = builder.ins().stack_addr(types::I64, tmp_obj, 0);
                    let idx_addr = builder.ins().stack_addr(types::I64, tmp_idx, 0);
                    let val_addr = builder.ins().stack_addr(types::I64, tmp_val, 0);
                    let out_addr = builder.ins().stack_addr(types::I64, tmp_out, 0);
                    builder.ins().store(memflags, obj[0], obj_addr, 0);
                    builder.ins().store(memflags, obj[1], obj_addr, 8);
                    builder.ins().store(memflags, idx[0], idx_addr, 0);
                    builder.ins().store(memflags, idx[1], idx_addr, 8);
                    builder.ins().store(memflags, val[0], val_addr, 0);
                    builder.ins().store(memflags, val[1], val_addr, 8);
                    builder.ins().store(memflags, val[2], val_addr, 16);
                    builder.ins().call(
                        ctx.func_refs.store_subscr,
                        &[obj_addr, idx_addr, val_addr, out_addr],
                    );
                }
                Opcode::MAP_ADD => {
                    // vm.rs's MAP_ADD: pop val, pop key, PEEK (not pop) the
                    // dict `arg` slots down (it stays on the stack — either
                    // for the next comprehension iteration, or as the
                    // literal dict's own final expression value once the
                    // compiler's trailing POP_TOPs clean up the DUP_TOP
                    // copies underneath — see compiler.rs's `Expr::Dict`
                    // codegen). `jit_store_subscr` already implements
                    // exactly `py_setitem(map, key, val)`, identical to
                    // MAP_ADD's own `PyObject::Dict::set` for a bare dict
                    // (no user `__setitem__` override is reachable on a
                    // just-`BUILD_MAP`-created object), so it's reused
                    // as-is rather than adding a near-duplicate helper.
                    let val = eval_stack.pop().unwrap();
                    let key = eval_stack.pop().unwrap();
                    let map = eval_stack[eval_stack.len() - 1 - instr.arg as usize];
                    let memflags = cranelift::codegen::ir::MemFlags::new();
                    let tmp_map = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot,
                        24,
                        0,
                    ));
                    let tmp_key = builder.create_sized_stack_slot(StackSlotData::new(
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
                    let map_addr = builder.ins().stack_addr(types::I64, tmp_map, 0);
                    let key_addr = builder.ins().stack_addr(types::I64, tmp_key, 0);
                    let val_addr = builder.ins().stack_addr(types::I64, tmp_val, 0);
                    let out_addr = builder.ins().stack_addr(types::I64, tmp_out, 0);
                    builder.ins().store(memflags, map[0], map_addr, 0);
                    builder.ins().store(memflags, map[1], map_addr, 8);
                    builder.ins().store(memflags, key[0], key_addr, 0);
                    builder.ins().store(memflags, key[1], key_addr, 8);
                    builder.ins().store(memflags, val[0], val_addr, 0);
                    builder.ins().store(memflags, val[1], val_addr, 8);
                    builder.ins().store(memflags, val[2], val_addr, 16);
                    builder.ins().call(
                        ctx.func_refs.store_subscr,
                        &[map_addr, key_addr, val_addr, out_addr],
                    );
                }
                Opcode::IS_OP => {
                    let invert = instr.arg as i64;
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
                    let invert_val = builder.ins().iconst(types::I64, invert);
                    builder
                        .ins()
                        .call(ctx.func_refs.is_op, &[a_addr, b_addr, invert_val, out_addr]);
                    let res_lo = builder.ins().load(types::I64, memflags, out_addr, 0);
                    let res_hi = builder.ins().load(types::I64, memflags, out_addr, 8);
                    let res_mid = builder.ins().load(types::I64, memflags, out_addr, 16);
                    eval_stack.push([res_lo, res_hi, res_mid]);
                }
                Opcode::UNARY_INVERT => {
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
                    builder.ins().call(ctx.func_refs.invert, &[val_addr, out_addr]);
                    let res_lo = builder.ins().load(types::I64, memflags, out_addr, 0);
                    let res_hi = builder.ins().load(types::I64, memflags, out_addr, 8);
                    let res_mid = builder.ins().load(types::I64, memflags, out_addr, 16);
                    eval_stack.push([res_lo, res_hi, res_mid]);
                }
                Opcode::IMPORT_NAME => {
                    let name_idx = instr.arg as i64;
                    let names_offset = (ctx.code.consts.len() + ctx.code.names.len()) as i64;
                    let memflags = cranelift::codegen::ir::MemFlags::new();
                    let tmp_out = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot,
                        24,
                        0,
                    ));
                    let out_addr = builder.ins().stack_addr(types::I64, tmp_out, 0);
                    let names_offset_val = builder.ins().iconst(types::I64, names_offset);
                    let name_idx_val = builder.ins().iconst(types::I64, name_idx);
                    builder.ins().call(
                        ctx.func_refs.import_name,
                        &[ctx.consts_ptr, names_offset_val, name_idx_val, out_addr],
                    );
                    let res_lo = builder.ins().load(types::I64, memflags, out_addr, 0);
                    let res_hi = builder.ins().load(types::I64, memflags, out_addr, 8);
                    let res_mid = builder.ins().load(types::I64, memflags, out_addr, 16);
                    eval_stack.push([res_lo, res_hi, res_mid]);
                }
                Opcode::IMPORT_FROM => {
                    let name_idx = instr.arg as i64;
                    let names_offset = (ctx.code.consts.len() + ctx.code.names.len()) as i64;
                    let module = *eval_stack.last().unwrap(); // peek, don't pop
                    let memflags = cranelift::codegen::ir::MemFlags::new();
                    let tmp_module = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot,
                        24,
                        0,
                    ));
                    let tmp_out = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot,
                        24,
                        0,
                    ));
                    let module_addr = builder.ins().stack_addr(types::I64, tmp_module, 0);
                    let out_addr = builder.ins().stack_addr(types::I64, tmp_out, 0);
                    builder.ins().store(memflags, module[0], module_addr, 0);
                    builder.ins().store(memflags, module[1], module_addr, 8);
                    let names_offset_val = builder.ins().iconst(types::I64, names_offset);
                    let name_idx_val = builder.ins().iconst(types::I64, name_idx);
                    builder.ins().call(
                        ctx.func_refs.import_from,
                        &[
                            module_addr,
                            ctx.consts_ptr,
                            names_offset_val,
                            name_idx_val,
                            out_addr,
                        ],
                    );
                    let res_lo = builder.ins().load(types::I64, memflags, out_addr, 0);
                    let res_hi = builder.ins().load(types::I64, memflags, out_addr, 8);
                    let res_mid = builder.ins().load(types::I64, memflags, out_addr, 16);
                    eval_stack.push([res_lo, res_hi, res_mid]);
                }
                Opcode::UNPACK_EX => {
                    let n_before = instr.arg & 0xFF;
                    let n_after = (instr.arg >> 8) & 0xFF;
                    let n_total = n_before + 1 + n_after; // before + starred + after
                    let seq = eval_stack.pop().unwrap();
                    let memflags = cranelift::codegen::ir::MemFlags::new();
                    let tmp_seq = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot,
                        24,
                        0,
                    ));
                    let array_size = ((n_total as usize * 24).max(16)) as u32;
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
                    let seq_addr = builder.ins().stack_addr(types::I64, tmp_seq, 0);
                    let array_addr = builder.ins().stack_addr(types::I64, array_slot, 0);
                    let out_addr = builder.ins().stack_addr(types::I64, tmp_out, 0);
                    builder.ins().store(memflags, seq[0], seq_addr, 0);
                    builder.ins().store(memflags, seq[1], seq_addr, 8);
                    let n_before_val = builder.ins().iconst(types::I64, n_before as i64);
                    let n_after_val = builder.ins().iconst(types::I64, n_after as i64);
                    builder.ins().call(
                        ctx.func_refs.unpack_ex,
                        &[seq_addr, n_before_val, n_after_val, array_addr, out_addr],
                    );
                    // Push unpacked items onto stack: items before *, starred list, items after *
                    for i in 0..n_total {
                        let offset = (i * 24) as i32;
                        let item_addr = builder.ins().iadd_imm(array_addr, offset as i64);
                        let ilo = builder.ins().load(types::I64, memflags, item_addr, 0);
                        let ihi = builder.ins().load(types::I64, memflags, item_addr, 8);
                        let imid = builder.ins().load(types::I64, memflags, item_addr, 16);
                        eval_stack.push([ilo, ihi, imid]);
                    }
                }
                Opcode::SETUP_WITH => {
                    let mgr = eval_stack.last().unwrap(); // peek, don't pop
                    let memflags = cranelift::codegen::ir::MemFlags::new();
                    let tmp_mgr = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot,
                        24,
                        0,
                    ));
                    let tmp_out = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot,
                        24,
                        0,
                    ));
                    let mgr_addr = builder.ins().stack_addr(types::I64, tmp_mgr, 0);
                    let out_addr = builder.ins().stack_addr(types::I64, tmp_out, 0);
                    builder.ins().store(memflags, mgr[0], mgr_addr, 0);
                    builder.ins().store(memflags, mgr[1], mgr_addr, 8);
                    builder
                        .ins()
                        .call(ctx.func_refs.setup_with, &[mgr_addr, out_addr]);
                    let res_lo = builder.ins().load(types::I64, memflags, out_addr, 0);
                    let res_hi = builder.ins().load(types::I64, memflags, out_addr, 8);
                    let res_mid = builder.ins().load(types::I64, memflags, out_addr, 16);
                    eval_stack.push([res_lo, res_hi, res_mid]);
                }
                Opcode::WITH_EXIT => {
                    let mgr = eval_stack.pop().unwrap();
                    let memflags = cranelift::codegen::ir::MemFlags::new();
                    let tmp_mgr = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot,
                        24,
                        0,
                    ));
                    let tmp_out = builder.create_sized_stack_slot(StackSlotData::new(
                        cranelift::codegen::ir::StackSlotKind::ExplicitSlot,
                        24,
                        0,
                    ));
                    let mgr_addr = builder.ins().stack_addr(types::I64, tmp_mgr, 0);
                    let out_addr = builder.ins().stack_addr(types::I64, tmp_out, 0);
                    builder.ins().store(memflags, mgr[0], mgr_addr, 0);
                    builder.ins().store(memflags, mgr[1], mgr_addr, 8);
                    builder
                        .ins()
                        .call(ctx.func_refs.with_exit, &[mgr_addr, out_addr]);
                    let res_lo = builder.ins().load(types::I64, memflags, out_addr, 0);
                    let res_hi = builder.ins().load(types::I64, memflags, out_addr, 8);
                    let res_mid = builder.ins().load(types::I64, memflags, out_addr, 16);
                    eval_stack.push([res_lo, res_hi, res_mid]);
                }
                Opcode::SETUP_FINALLY => {
                    // No-op: exception handlers are managed by the VM's interpreter.
                    // Any exceptions will propagate out of JIT-compiled ctx.code and be
                    // caught by the VM's exception handling at a higher level.
                }
                _ => {
                    eprintln!("JIT: codegen unsupported {:?} at instr {}", instr.op, ctx.i);
                    return false;
                }
            
        _ => return false,
    }
    true
}
