use cranelift::prelude::*;
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{Linkage, Module};
use crate::object::PyObjectRef;

/// JIT-compiled function signature:
/// fn(args: *const PyObjectRef, nargs: usize, consts: *const PyObjectRef, result: *mut PyObjectRef)
/// Returns nothing — writes result to *result.

extern "C" fn jit_py_add(a: *const PyObjectRef, b: *const PyObjectRef, out: *mut PyObjectRef) {
    unsafe { std::ptr::write(out, crate::object::py_add(&*a, &*b).unwrap_or_else(|_| crate::object::py_none())); }
}
extern "C" fn jit_py_sub(a: *const PyObjectRef, b: *const PyObjectRef, out: *mut PyObjectRef) {
    unsafe { std::ptr::write(out, crate::object::py_sub(&*a, &*b).unwrap_or_else(|_| crate::object::py_none())); }
}
extern "C" fn jit_py_mul(a: *const PyObjectRef, b: *const PyObjectRef, out: *mut PyObjectRef) {
    unsafe { std::ptr::write(out, crate::object::py_mul(&*a, &*b).unwrap_or_else(|_| crate::object::py_none())); }
}

pub struct JitCompiler {
    builder_context: FunctionBuilderContext,
    module: JITModule,
    add_func: cranelift_module::FuncId,
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
        let mut module = JITModule::new(builder);
        let binop_sig = Self::make_binop_sig();
        let add_func = module.declare_function("jit_py_add", Linkage::Import, &binop_sig).unwrap();
        JitCompiler {
            builder_context: FunctionBuilderContext::new(),
            module,
            add_func,
        }
    }

    fn make_binop_sig() -> cranelift::codegen::ir::Signature {
        let mut s = cranelift::codegen::ir::Signature::new(cranelift::codegen::isa::CallConv::SystemV);
        s.params.push(AbiParam::new(types::I64));
        s.params.push(AbiParam::new(types::I64));
        s.params.push(AbiParam::new(types::I64));
        s
    }

    /// Build a signature for binary ops: fn(*const PyObjectRef, *const PyObjectRef, *mut PyObjectRef)
    pub fn is_enabled() -> bool { false } // Enable when more opcodes are supported

    pub fn precompute_consts(code: &crate::bytecode::CodeObject) -> Vec<PyObjectRef> {
        use crate::bytecode::ConstValue;
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

    pub fn compile(
        &mut self,
        code: &crate::bytecode::CodeObject,
    ) -> Option<extern "C" fn(*const PyObjectRef, usize, *const PyObjectRef, *mut PyObjectRef)> {
        if !Self::is_enabled() { return None; }
        // Don't JIT functions with *args or **kwargs (handled specially by VM)
        if code.vararg_name.is_some() || code.kwarg_name.is_some() { return None; }
        if code.instructions.is_empty() || code.instructions.len() > 100 {
            return None;
        }
        let supported: &[crate::bytecode::Opcode] = &[
            crate::bytecode::Opcode::LOAD_FAST, crate::bytecode::Opcode::LOAD_CONST,
            crate::bytecode::Opcode::RETURN_VALUE, crate::bytecode::Opcode::STORE_FAST,
            crate::bytecode::Opcode::DUP_TOP, crate::bytecode::Opcode::POP_TOP,
        ];
        // BINARY_OP not yet supported (needs cranelift FuncRef fix)
        for instr in &code.instructions {
            if !supported.contains(&instr.op) { return None; }
        }

        let _consts = Self::precompute_consts(code);

        // Build function signature: (args, nargs, consts, result) -> void
        let mut sig = cranelift::codegen::ir::Signature::new(cranelift::codegen::isa::CallConv::SystemV);
        sig.params.push(AbiParam::new(types::I64)); // args: *const PyObjectRef
        sig.params.push(AbiParam::new(types::I64)); // nargs: usize
        sig.params.push(AbiParam::new(types::I64)); // consts: *const PyObjectRef
        sig.params.push(AbiParam::new(types::I64)); // result: *mut PyObjectRef

        let mut ctx = cranelift::codegen::Context::new();
        ctx.func.signature = sig.clone();
        let func = self.module.declare_function("jit_fn", Linkage::Local, &sig).ok()?;
        let mut builder = FunctionBuilder::new(&mut ctx.func, &mut self.builder_context);

        let entry = builder.create_block();
        builder.append_block_params_for_function_params(entry);
        builder.switch_to_block(entry);
        builder.seal_block(entry);

        let args_ptr = builder.block_params(entry)[0];
        let _nargs = builder.block_params(entry)[1];
        let consts_ptr = builder.block_params(entry)[2];
        let result_ptr = builder.block_params(entry)[3];

        // Allocate locals array on stack (16 bytes per PyObjectRef, up to nlocals slots)
        let locals_size = (code.nlocals.max(1) * 16) as u32;
        let locals_slot = builder.create_sized_stack_slot(StackSlotData::new(
            StackSlotKind::ExplicitSlot, locals_size, 0,
        ));

        // Copy args to locals
        for i in 0..code.arg_count.min(code.nlocals) {
            let src = builder.ins().iadd_imm(args_ptr, (i * 16) as i64);
            let dst = builder.ins().stack_addr(types::I64, locals_slot, (i * 16) as i32);
            let lo = builder.ins().load(types::I64, cranelift::codegen::ir::MemFlags::trusted(), src, 0);
            let hi = builder.ins().load(types::I64, cranelift::codegen::ir::MemFlags::trusted(), src, 8);
            builder.ins().store(cranelift::codegen::ir::MemFlags::trusted(), lo, dst, 0);
            builder.ins().store(cranelift::codegen::ir::MemFlags::trusted(), hi, dst, 8);
        }

        // Evaluation stack — allocate as stack memory
        let stack_slot = builder.create_sized_stack_slot(StackSlotData::new(
            StackSlotKind::ExplicitSlot, 256, 0, // 16 slots × 16 bytes
        ));
        let mut sp: i32 = 0; // stack pointer in bytes

        // Generate code for each instruction
        for instr in &code.instructions {
            match instr.op {
                crate::bytecode::Opcode::LOAD_FAST => {
                    let idx = instr.arg as i32;
                    let src = builder.ins().stack_addr(types::I64, locals_slot, idx * 16);
                    let dst = builder.ins().stack_addr(types::I64, stack_slot, sp);
                    let lo = builder.ins().load(types::I64, cranelift::codegen::ir::MemFlags::trusted(), src, 0);
                    let hi = builder.ins().load(types::I64, cranelift::codegen::ir::MemFlags::trusted(), src, 8);
                    builder.ins().store(cranelift::codegen::ir::MemFlags::trusted(), lo, dst, 0);
                    builder.ins().store(cranelift::codegen::ir::MemFlags::trusted(), hi, dst, 8);
                    sp += 16;
                }
                crate::bytecode::Opcode::LOAD_CONST => {
                    let idx = instr.arg as i32;
                    let src = builder.ins().iadd_imm(consts_ptr, (idx * 16) as i64);
                    let dst = builder.ins().stack_addr(types::I64, stack_slot, sp);
                    let lo = builder.ins().load(types::I64, cranelift::codegen::ir::MemFlags::trusted(), src, 0);
                    let hi = builder.ins().load(types::I64, cranelift::codegen::ir::MemFlags::trusted(), src, 8);
                    builder.ins().store(cranelift::codegen::ir::MemFlags::trusted(), lo, dst, 0);
                    builder.ins().store(cranelift::codegen::ir::MemFlags::trusted(), hi, dst, 8);
                    sp += 16;
                }
                crate::bytecode::Opcode::BINARY_OP => {
                    // BINARY_OP not yet supported (cranelift FuncRef issue)
                    // For now, fall back by returning None (not reaching here due to supported check)
                }
                crate::bytecode::Opcode::STORE_FAST => {
                    sp -= 16;
                    let idx = instr.arg as i32;
                    let src = builder.ins().stack_addr(types::I64, stack_slot, sp);
                    let dst = builder.ins().stack_addr(types::I64, locals_slot, idx * 16);
                    let lo = builder.ins().load(types::I64, cranelift::codegen::ir::MemFlags::trusted(), src, 0);
                    let hi = builder.ins().load(types::I64, cranelift::codegen::ir::MemFlags::trusted(), src, 8);
                    builder.ins().store(cranelift::codegen::ir::MemFlags::trusted(), lo, dst, 0);
                    builder.ins().store(cranelift::codegen::ir::MemFlags::trusted(), hi, dst, 8);
                }
                crate::bytecode::Opcode::DUP_TOP => {
                    let src = builder.ins().stack_addr(types::I64, stack_slot, sp - 16);
                    let dst = builder.ins().stack_addr(types::I64, stack_slot, sp);
                    let lo = builder.ins().load(types::I64, cranelift::codegen::ir::MemFlags::trusted(), src, 0);
                    let hi = builder.ins().load(types::I64, cranelift::codegen::ir::MemFlags::trusted(), src, 8);
                    builder.ins().store(cranelift::codegen::ir::MemFlags::trusted(), lo, dst, 0);
                    builder.ins().store(cranelift::codegen::ir::MemFlags::trusted(), hi, dst, 8);
                    sp += 16;
                }
                crate::bytecode::Opcode::POP_TOP => {
                    sp -= 16;
                }
                crate::bytecode::Opcode::RETURN_VALUE => {
                    sp -= 16;
                    let src = builder.ins().stack_addr(types::I64, stack_slot, sp);
                    let lo = builder.ins().load(types::I64, cranelift::codegen::ir::MemFlags::trusted(), src, 0);
                    let hi = builder.ins().load(types::I64, cranelift::codegen::ir::MemFlags::trusted(), src, 8);
                    builder.ins().store(cranelift::codegen::ir::MemFlags::trusted(), lo, result_ptr, 0);
                    builder.ins().store(cranelift::codegen::ir::MemFlags::trusted(), hi, result_ptr, 8);
                    builder.ins().return_(&[]);
                    builder.finalize();
                    self.module.define_function(func, &mut ctx).ok()?;
                    self.module.finalize_definitions().ok()?;
                    let code_ptr = self.module.get_finalized_function(func);
                    if code_ptr.is_null() { return None; }
                    return Some(unsafe { std::mem::transmute(code_ptr) });
                }
                _ => return None,
            }
        }

        // If no RETURN_VALUE found, result is already py_none (set by caller)
        builder.ins().return_(&[]);
        builder.finalize();
        self.module.define_function(func, &mut ctx).ok()?;
        self.module.finalize_definitions().ok()?;
        let code_ptr = self.module.get_finalized_function(func);
        if code_ptr.is_null() { return None; }
        Some(unsafe { std::mem::transmute(code_ptr) })
    }
}
