use std::collections::HashSet;
use std::collections::HashMap;
use cranelift::prelude::*;
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{Linkage, Module};
use crate::object::PyObjectRef;
use crate::bytecode::*;

extern "C" fn jit_py_add(a: *const PyObjectRef, b: *const PyObjectRef, out: *mut PyObjectRef) {
    unsafe { std::ptr::write(out, crate::object::py_add(&*a, &*b).unwrap_or_else(|_| crate::object::py_none())); }
}
extern "C" fn jit_py_sub(a: *const PyObjectRef, b: *const PyObjectRef, out: *mut PyObjectRef) {
    unsafe { std::ptr::write(out, crate::object::py_sub(&*a, &*b).unwrap_or_else(|_| crate::object::py_none())); }
}
extern "C" fn jit_py_mul(a: *const PyObjectRef, b: *const PyObjectRef, out: *mut PyObjectRef) {
    unsafe { std::ptr::write(out, crate::object::py_mul(&*a, &*b).unwrap_or_else(|_| crate::object::py_none())); }
}
extern "C" fn jit_py_compare(a: *const PyObjectRef, b: *const PyObjectRef, op: i64, out: *mut PyObjectRef) {
    unsafe { std::ptr::write(out, crate::object::py_compare(&*a, &*b, op as u32).unwrap_or_else(|_| crate::object::py_bool(false))); }
}
extern "C" fn jit_is_true(val: *const PyObjectRef) -> i64 {
    unsafe { (*val).truthy() as i64 }
}

pub struct JitCompiler {
    builder_context: FunctionBuilderContext,
    module: JITModule,
    add_func: cranelift_module::FuncId,
    sub_func: cranelift_module::FuncId,
    mul_func: cranelift_module::FuncId,
    cmp_func: cranelift_module::FuncId,
    truthy_func: cranelift_module::FuncId,
}

impl JitCompiler {
    pub fn new() -> Self {
        let flag_builder = settings::builder();
        let flags = settings::Flags::new(flag_builder);
        let isa_builder = cranelift_native::builder().unwrap();
        let isa = isa_builder.finish(flags).unwrap();
        let mut builder = JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());
        builder.symbol("jit_py_add", jit_py_add as *const u8);
        builder.symbol("jit_py_sub", jit_py_sub as *const u8);
        builder.symbol("jit_py_mul", jit_py_mul as *const u8);
        builder.symbol("jit_py_compare", jit_py_compare as *const u8);
        builder.symbol("jit_is_true", jit_is_true as *const u8);
        let mut module = JITModule::new(builder);
        let binop_sig = Self::make_binop_sig();
        let cmp_sig = Self::make_cmp_sig();
        let truthy_sig = Self::make_truthy_sig();
        let add_func = module.declare_function("jit_py_add", Linkage::Import, &binop_sig).unwrap();
        let sub_func = module.declare_function("jit_py_sub", Linkage::Import, &binop_sig).unwrap();
        let mul_func = module.declare_function("jit_py_mul", Linkage::Import, &binop_sig).unwrap();
        let cmp_func = module.declare_function("jit_py_compare", Linkage::Import, &cmp_sig).unwrap();
        let truthy_func = module.declare_function("jit_is_true", Linkage::Import, &truthy_sig).unwrap();
        JitCompiler {
            builder_context: FunctionBuilderContext::new(),
            module,
            add_func,
            sub_func,
            mul_func,
            cmp_func,
            truthy_func,
        }
    }

    fn make_binop_sig() -> cranelift::codegen::ir::Signature {
        let mut s = cranelift::codegen::ir::Signature::new(cranelift::codegen::isa::CallConv::SystemV);
        s.params.push(AbiParam::new(types::I64));
        s.params.push(AbiParam::new(types::I64));
        s.params.push(AbiParam::new(types::I64));
        s
    }

    fn make_cmp_sig() -> cranelift::codegen::ir::Signature {
        let mut s = cranelift::codegen::ir::Signature::new(cranelift::codegen::isa::CallConv::SystemV);
        s.params.push(AbiParam::new(types::I64));
        s.params.push(AbiParam::new(types::I64));
        s.params.push(AbiParam::new(types::I64));
        s.params.push(AbiParam::new(types::I64));
        s
    }

    fn make_truthy_sig() -> cranelift::codegen::ir::Signature {
        let mut s = cranelift::codegen::ir::Signature::new(cranelift::codegen::isa::CallConv::SystemV);
        s.params.push(AbiParam::new(types::I64));
        s.returns.push(AbiParam::new(types::I64));
        s
    }

    pub fn is_enabled() -> bool { true }

    pub fn precompute_consts(code: &CodeObject) -> Vec<PyObjectRef> {
        code.consts.iter().map(|cv| match cv {
            ConstValue::None => crate::object::py_none(),
            ConstValue::Bool(b) => crate::object::py_bool(*b),
            ConstValue::Int(s) => {
                if let Ok(n) = s.parse::<i64>() { crate::object::py_int(n) }
                else { crate::object::PyObjectRef::new(crate::object::PyObject::Int(s.parse().unwrap())) }
            }
            ConstValue::Float(f) => crate::object::py_float(f.parse().unwrap_or(0.0)),
            ConstValue::String(s) => crate::object::py_str(s),
            ConstValue::Bytes(b) => crate::object::PyObjectRef::new(crate::object::PyObject::Bytes(b.clone())),
            ConstValue::Code(_) => crate::object::py_none(),
        }).collect()
    }

    /// Build a Cranelift function that implements the given bytecode.
    /// Supports straight-line code, loops (via JUMP_BACKWARD), and conditional branches.
    pub fn compile(
        &mut self,
        code: &CodeObject,
    ) -> Option<extern "C" fn(*const PyObjectRef, usize, *const PyObjectRef, *mut PyObjectRef)> {
        if !Self::is_enabled() { return None; }
        if code.vararg_name.is_some() || code.kwarg_name.is_some() || code.kwonlyarg_count > 0 || code.num_defaults > 0 { return None; }
        if code.instructions.is_empty() || code.instructions.len() > 200 {
            return None;
        }

        let supported: &[Opcode] = &[
            Opcode::LOAD_FAST, Opcode::LOAD_CONST,
            Opcode::BINARY_OP, Opcode::RETURN_VALUE,
            Opcode::STORE_FAST, Opcode::DUP_TOP,
            Opcode::POP_TOP, Opcode::COMPARE_OP,
            Opcode::POP_JUMP_IF_FALSE, Opcode::JUMP_BACKWARD,
        ];
        for instr in &code.instructions {
            if !supported.contains(&instr.op) { return None; }
        }

        let _consts = Self::precompute_consts(code);

        let mut sig = cranelift::codegen::ir::Signature::new(cranelift::codegen::isa::CallConv::SystemV);
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));

        let mut ctx = cranelift::codegen::Context::new();
        ctx.func.signature = sig.clone();
        let func = self.module.declare_function("jit_fn", Linkage::Local, &sig).ok()?;

        let add_func_ref = self.module.declare_func_in_func(self.add_func, &mut ctx.func);
        let sub_func_ref = self.module.declare_func_in_func(self.sub_func, &mut ctx.func);
        let mul_func_ref = self.module.declare_func_in_func(self.mul_func, &mut ctx.func);
        let cmp_func_ref = self.module.declare_func_in_func(self.cmp_func, &mut ctx.func);
        let truthy_func_ref = self.module.declare_func_in_func(self.truthy_func, &mut ctx.func);

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
                    let target = i.wrapping_sub(instr.arg as usize).wrapping_sub(1);
                    targets.insert(target);
                    targets.insert(i + 1);
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
        let locals_size = (code.nlocals.max(1) * 16) as u32;
        let locals_slot = builder.create_sized_stack_slot(StackSlotData::new(
            StackSlotKind::ExplicitSlot, locals_size, 0,
        ));

        // Copy args to locals
        for i in 0..code.arg_count.min(code.nlocals) {
            let src = builder.ins().iadd_imm(args_ptr, (i * 16) as i64);
            let dst = builder.ins().stack_addr(types::I64, locals_slot, (i * 16) as i32);
            let lo = builder.ins().load(types::I64, cranelift::codegen::ir::MemFlags::new(), src, 0);
            let hi = builder.ins().load(types::I64, cranelift::codegen::ir::MemFlags::new(), src, 8);
            builder.ins().store(cranelift::codegen::ir::MemFlags::new(), lo, dst, 0);
            builder.ins().store(cranelift::codegen::ir::MemFlags::new(), hi, dst, 8);
        }

        // Evaluation stack
        let stack_slot = builder.create_sized_stack_slot(StackSlotData::new(
            StackSlotKind::ExplicitSlot, 256, 0,
        ));
        let mut sp: i32 = 0;

        // Generate code for each instruction
        for i in 0..code.instructions.len() {
            let block = instr_to_block[&i];

            // Switch to the correct block if not already there
            if builder.current_block() != Some(block) {
                // Switch to the new block
                builder.switch_to_block(block);
                blocks_entered.insert(block);
            }

            let instr = &code.instructions[i];
            match instr.op {
                Opcode::LOAD_FAST => {
                    let idx = instr.arg as i32;
                    let src = builder.ins().stack_addr(types::I64, locals_slot, idx * 16);
                    let dst = builder.ins().stack_addr(types::I64, stack_slot, sp);
                    let lo = builder.ins().load(types::I64, cranelift::codegen::ir::MemFlags::new(), src, 0);
                    let hi = builder.ins().load(types::I64, cranelift::codegen::ir::MemFlags::new(), src, 8);
                    builder.ins().store(cranelift::codegen::ir::MemFlags::new(), lo, dst, 0);
                    builder.ins().store(cranelift::codegen::ir::MemFlags::new(), hi, dst, 8);
                    sp += 16;
                }
                Opcode::LOAD_CONST => {
                    let idx = instr.arg as i32;
                    let src = builder.ins().iadd_imm(consts_ptr, (idx * 16) as i64);
                    let dst = builder.ins().stack_addr(types::I64, stack_slot, sp);
                    let lo = builder.ins().load(types::I64, cranelift::codegen::ir::MemFlags::new(), src, 0);
                    let hi = builder.ins().load(types::I64, cranelift::codegen::ir::MemFlags::new(), src, 8);
                    builder.ins().store(cranelift::codegen::ir::MemFlags::new(), lo, dst, 0);
                    builder.ins().store(cranelift::codegen::ir::MemFlags::new(), hi, dst, 8);
                    sp += 16;
                }
                Opcode::BINARY_OP => {
                    sp -= 32;
                    let a_addr = builder.ins().stack_addr(types::I64, stack_slot, sp);
                    let b_addr = builder.ins().stack_addr(types::I64, stack_slot, sp + 16);
                    let out_addr = builder.ins().stack_addr(types::I64, stack_slot, sp);
                    let func_ref = match instr.arg {
                        0 => add_func_ref,
                        1 => sub_func_ref,
                        2 => mul_func_ref,
                        _ => return None,
                    };
                    builder.ins().call(func_ref, &[a_addr, b_addr, out_addr]);
                    sp += 16;
                }
                Opcode::COMPARE_OP => {
                    sp -= 32;
                    let a_addr = builder.ins().stack_addr(types::I64, stack_slot, sp);
                    let b_addr = builder.ins().stack_addr(types::I64, stack_slot, sp + 16);
                    let out_addr = builder.ins().stack_addr(types::I64, stack_slot, sp);
                    let op_val = builder.ins().iconst(types::I64, instr.arg as i64);
                    builder.ins().call(cmp_func_ref, &[a_addr, b_addr, op_val, out_addr]);
                    sp += 16;
                }
                Opcode::STORE_FAST => {
                    sp -= 16;
                    let idx = instr.arg as i32;
                    let src = builder.ins().stack_addr(types::I64, stack_slot, sp);
                    let dst = builder.ins().stack_addr(types::I64, locals_slot, idx * 16);
                    let lo = builder.ins().load(types::I64, cranelift::codegen::ir::MemFlags::new(), src, 0);
                    let hi = builder.ins().load(types::I64, cranelift::codegen::ir::MemFlags::new(), src, 8);
                    builder.ins().store(cranelift::codegen::ir::MemFlags::new(), lo, dst, 0);
                    builder.ins().store(cranelift::codegen::ir::MemFlags::new(), hi, dst, 8);
                }
                Opcode::DUP_TOP => {
                    let src = builder.ins().stack_addr(types::I64, stack_slot, sp - 16);
                    let dst = builder.ins().stack_addr(types::I64, stack_slot, sp);
                    let lo = builder.ins().load(types::I64, cranelift::codegen::ir::MemFlags::new(), src, 0);
                    let hi = builder.ins().load(types::I64, cranelift::codegen::ir::MemFlags::new(), src, 8);
                    builder.ins().store(cranelift::codegen::ir::MemFlags::new(), lo, dst, 0);
                    builder.ins().store(cranelift::codegen::ir::MemFlags::new(), hi, dst, 8);
                    sp += 16;
                }
                Opcode::POP_TOP => {
                    sp -= 16;
                }
                Opcode::POP_JUMP_IF_FALSE => {
                    sp -= 16;
                    let val_addr = builder.ins().stack_addr(types::I64, stack_slot, sp);
                    let truthy_inst = builder.ins().call(truthy_func_ref, &[val_addr]);
                    let truthy = builder.inst_results(truthy_inst)[0];
                    let zero = builder.ins().iconst(types::I64, 0);
                    let cmp = builder.ins().icmp(IntCC::Equal, truthy, zero);

                    let target = instr.arg as usize;
                    let target_block = block_of[&target];
                    let next_block = block_of[&(i + 1)];

                    builder.ins().brif(cmp, target_block, &[], next_block, &[]);
                }
                Opcode::JUMP_BACKWARD => {
                    let target = i.wrapping_sub(instr.arg as usize).wrapping_sub(1);
                    let target_block = block_of[&target];
                    builder.ins().jump(target_block, &[]);
                }
                Opcode::RETURN_VALUE => {
                    sp -= 16;
                    let src = builder.ins().stack_addr(types::I64, stack_slot, sp);
                    let lo = builder.ins().load(types::I64, cranelift::codegen::ir::MemFlags::new(), src, 0);
                    let hi = builder.ins().load(types::I64, cranelift::codegen::ir::MemFlags::new(), src, 8);
                    builder.ins().store(cranelift::codegen::ir::MemFlags::new(), lo, result_ptr, 0);
                    builder.ins().store(cranelift::codegen::ir::MemFlags::new(), hi, result_ptr, 8);
                    builder.ins().return_(&[]);
                }
                _ => return None,
            }
        }

        // Seal remaining unsealed blocks
        for &idx in &sorted_targets {
            let block = block_of[&idx];
            if blocks_entered.contains(&block) {
                builder.seal_block(block);
            }
        }

        builder.seal_all_blocks();

        builder.finalize();
        self.module.define_function(func, &mut ctx).ok()?;
        self.module.finalize_definitions().ok()?;
        let code_ptr = self.module.get_finalized_function(func);
        if code_ptr.is_null() { return None; }
        Some(unsafe { std::mem::transmute(code_ptr) })
    }
}
