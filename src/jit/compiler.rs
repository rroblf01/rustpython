use crate::bytecode::CodeObject;
use crate::object::PyObjectRef;
use cranelift::prelude::*;
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{Linkage, Module};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use super::runtime::*;
use super::runtime_extra::*;

pub struct JitCompiler {
    pub(crate) builder_context: FunctionBuilderContext,
    pub(crate) module: JITModule,
    pub(crate) add_func: cranelift_module::FuncId,
    pub(crate) sub_func: cranelift_module::FuncId,
    pub(crate) mul_func: cranelift_module::FuncId,
    pub(crate) div_func: cranelift_module::FuncId,
    pub(crate) floor_div_func: cranelift_module::FuncId,
    pub(crate) mod_func: cranelift_module::FuncId,
    pub(crate) pow_func: cranelift_module::FuncId,
    pub(crate) lshift_func: cranelift_module::FuncId,
    pub(crate) rshift_func: cranelift_module::FuncId,
    pub(crate) bit_and_func: cranelift_module::FuncId,
    pub(crate) bit_or_func: cranelift_module::FuncId,
    pub(crate) bit_xor_func: cranelift_module::FuncId,
    pub(crate) inplace_binop_func: cranelift_module::FuncId,
    pub(crate) getitem_func: cranelift_module::FuncId,
    pub(crate) cmp_func: cranelift_module::FuncId,
    pub(crate) truthy_func: cranelift_module::FuncId,
    pub(crate) neg_func: cranelift_module::FuncId,
    pub(crate) not_func: cranelift_module::FuncId,
    pub(crate) build_list_func: cranelift_module::FuncId,
    pub(crate) build_tuple_func: cranelift_module::FuncId,
    pub(crate) list_append_func: cranelift_module::FuncId,
    pub(crate) contains_func: cranelift_module::FuncId,
    pub(crate) get_iter_func: cranelift_module::FuncId,
    pub(crate) call_func: cranelift_module::FuncId,
    pub(crate) call_kw_func: cranelift_module::FuncId,
    pub(crate) load_attr_func: cranelift_module::FuncId,
    pub(crate) for_iter_func: cranelift_module::FuncId,
    pub(crate) build_map_func: cranelift_module::FuncId,
    pub(crate) store_attr_func: cranelift_module::FuncId,
    pub(crate) unpack_sequence_func: cranelift_module::FuncId,
    pub(crate) load_name_func: cranelift_module::FuncId,
    pub(crate) build_set_func: cranelift_module::FuncId,
    pub(crate) build_string_func: cranelift_module::FuncId,
    pub(crate) build_slice_func: cranelift_module::FuncId,
    pub(crate) store_subscr_func: cranelift_module::FuncId,
    pub(crate) is_op_func: cranelift_module::FuncId,
    pub(crate) invert_func: cranelift_module::FuncId,
    pub(crate) import_name_func: cranelift_module::FuncId,
    pub(crate) import_from_func: cranelift_module::FuncId,
    pub(crate) unpack_ex_func: cranelift_module::FuncId,
    pub(crate) setup_with_func: cranelift_module::FuncId,
    pub(crate) with_exit_func: cranelift_module::FuncId,
    pub(crate) make_function_func: cranelift_module::FuncId,
}

impl JitCompiler {
    pub fn new() -> Self {
        let flag_builder = settings::builder();
        #[cfg(debug_assertions)]
        let flag_builder = settings::builder();
        let isa_builder = cranelift_native::builder().unwrap();
        let isa = isa_builder
            .finish(settings::Flags::new(flag_builder))
            .unwrap();
        let mut builder = JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());
        builder.symbol("jit_py_add", jit_py_add as *const u8);
        builder.symbol("jit_py_sub", jit_py_sub as *const u8);
        builder.symbol("jit_py_mul", jit_py_mul as *const u8);
        builder.symbol("jit_py_div", jit_py_div as *const u8);
        builder.symbol("jit_py_floor_div", jit_py_floor_div as *const u8);
        builder.symbol("jit_py_mod", jit_py_mod as *const u8);
        builder.symbol("jit_py_pow", jit_py_pow as *const u8);
        builder.symbol("jit_py_lshift", jit_py_lshift as *const u8);
        builder.symbol("jit_py_rshift", jit_py_rshift as *const u8);
        builder.symbol("jit_py_bit_and", jit_py_bit_and as *const u8);
        builder.symbol("jit_py_bit_or", jit_py_bit_or as *const u8);
        builder.symbol("jit_py_bit_xor", jit_py_bit_xor as *const u8);
        builder.symbol("jit_py_inplace_binop", jit_py_inplace_binop as *const u8);
        builder.symbol("jit_getitem", jit_getitem as *const u8);
        builder.symbol("jit_py_compare", jit_py_compare as *const u8);
        builder.symbol("jit_is_true", jit_is_true as *const u8);
        builder.symbol("jit_neg", jit_neg as *const u8);
        builder.symbol("jit_not", jit_not as *const u8);
        builder.symbol("jit_build_list", jit_build_list as *const u8);
        builder.symbol("jit_build_tuple", jit_build_tuple as *const u8);
        builder.symbol("jit_list_append", jit_list_append as *const u8);
        builder.symbol("jit_contains", jit_contains as *const u8);
        builder.symbol("jit_get_iter", jit_get_iter as *const u8);
        builder.symbol("jit_call", jit_call as *const u8);
        builder.symbol("jit_call_kw", jit_call_kw as *const u8);
        builder.symbol("jit_load_attr", jit_load_attr as *const u8);
        builder.symbol("jit_for_iter", jit_for_iter as *const u8);
        builder.symbol("jit_build_map", jit_build_map as *const u8);
        builder.symbol("jit_store_attr", jit_store_attr as *const u8);
        builder.symbol("jit_unpack_sequence", jit_unpack_sequence as *const u8);
        builder.symbol("jit_load_name", jit_load_name as *const u8);
        builder.symbol("jit_build_set", jit_build_set as *const u8);
        builder.symbol("jit_build_string", jit_build_string as *const u8);
        builder.symbol("jit_build_slice", jit_build_slice as *const u8);
        builder.symbol("jit_store_subscr", jit_store_subscr as *const u8);
        builder.symbol("jit_is_op", jit_is_op as *const u8);
        builder.symbol("jit_invert", jit_invert as *const u8);
        builder.symbol("jit_import_name", jit_import_name as *const u8);
        builder.symbol("jit_import_from", jit_import_from as *const u8);
        builder.symbol("jit_unpack_ex", jit_unpack_ex as *const u8);
        builder.symbol("jit_setup_with", jit_setup_with as *const u8);
        builder.symbol("jit_with_exit", jit_with_exit as *const u8);
        builder.symbol("jit_make_function", jit_make_function as *const u8);
        let mut module = JITModule::new(builder);
        let binop_sig = Self::make_binop_sig();
        let cmp_sig = Self::make_cmp_sig();
        let truthy_sig = Self::make_truthy_sig();
        let unary_sig = Self::make_unary_sig();
        let call_sig = Self::make_call_sig();
        let load_attr_sig = Self::make_load_attr_sig();
        let store_attr_sig = Self::make_store_attr_sig();
        let unpack_sig = Self::make_unpack_sig();
        let import_sig = Self::make_import_sig();
        let import_from_sig = Self::make_import_from_sig();
        let unpack_ex_sig = Self::make_unpack_ex_sig();
        let context_sig = Self::make_context_sig();
        let make_function_sig = Self::make_make_function_sig();
        let add_func = module
            .declare_function("jit_py_add", Linkage::Import, &binop_sig)
            .unwrap();
        let sub_func = module
            .declare_function("jit_py_sub", Linkage::Import, &binop_sig)
            .unwrap();
        let mul_func = module
            .declare_function("jit_py_mul", Linkage::Import, &binop_sig)
            .unwrap();
        let div_func = module
            .declare_function("jit_py_div", Linkage::Import, &binop_sig)
            .unwrap();
        let floor_div_func = module
            .declare_function("jit_py_floor_div", Linkage::Import, &binop_sig)
            .unwrap();
        let mod_func = module
            .declare_function("jit_py_mod", Linkage::Import, &binop_sig)
            .unwrap();
        let pow_func = module
            .declare_function("jit_py_pow", Linkage::Import, &binop_sig)
            .unwrap();
        let lshift_func = module
            .declare_function("jit_py_lshift", Linkage::Import, &binop_sig)
            .unwrap();
        let rshift_func = module
            .declare_function("jit_py_rshift", Linkage::Import, &binop_sig)
            .unwrap();
        let bit_and_func = module
            .declare_function("jit_py_bit_and", Linkage::Import, &binop_sig)
            .unwrap();
        let bit_or_func = module
            .declare_function("jit_py_bit_or", Linkage::Import, &binop_sig)
            .unwrap();
        let bit_xor_func = module
            .declare_function("jit_py_bit_xor", Linkage::Import, &binop_sig)
            .unwrap();
        let inplace_binop_func = module
            .declare_function("jit_py_inplace_binop", Linkage::Import, &cmp_sig)
            .unwrap();
        let getitem_func = module
            .declare_function("jit_getitem", Linkage::Import, &binop_sig)
            .unwrap();
        let cmp_func = module
            .declare_function("jit_py_compare", Linkage::Import, &cmp_sig)
            .unwrap();
        let truthy_func = module
            .declare_function("jit_is_true", Linkage::Import, &truthy_sig)
            .unwrap();
        let neg_func = module
            .declare_function("jit_neg", Linkage::Import, &unary_sig)
            .unwrap();
        let not_func = module
            .declare_function("jit_not", Linkage::Import, &unary_sig)
            .unwrap();
        let build_list_func = module
            .declare_function("jit_build_list", Linkage::Import, &binop_sig)
            .unwrap();
        let build_tuple_func = module
            .declare_function("jit_build_tuple", Linkage::Import, &binop_sig)
            .unwrap();
        let list_append_func = module
            .declare_function("jit_list_append", Linkage::Import, &binop_sig)
            .unwrap();
        let contains_func = module
            .declare_function("jit_contains", Linkage::Import, &binop_sig)
            .unwrap();
        let get_iter_func = module
            .declare_function("jit_get_iter", Linkage::Import, &unary_sig)
            .unwrap();
        let call_func = module
            .declare_function("jit_call", Linkage::Import, &call_sig)
            .unwrap();
        let call_kw_sig = Self::make_call_kw_sig();
        let call_kw_func = module
            .declare_function("jit_call_kw", Linkage::Import, &call_kw_sig)
            .unwrap();
        let load_attr_func = module
            .declare_function("jit_load_attr", Linkage::Import, &load_attr_sig)
            .unwrap();
        let for_iter_func = module
            .declare_function("jit_for_iter", Linkage::Import, &truthy_sig)
            .unwrap();
        let build_map_func = module
            .declare_function("jit_build_map", Linkage::Import, &call_sig)
            .unwrap();
        let store_attr_func = module
            .declare_function("jit_store_attr", Linkage::Import, &store_attr_sig)
            .unwrap();
        let unpack_sequence_func = module
            .declare_function("jit_unpack_sequence", Linkage::Import, &unpack_sig)
            .unwrap();
        let load_name_func = module
            .declare_function("jit_load_name", Linkage::Import, &store_attr_sig)
            .unwrap();
        let build_set_func = module
            .declare_function("jit_build_set", Linkage::Import, &binop_sig)
            .unwrap();
        let build_string_func = module
            .declare_function("jit_build_string", Linkage::Import, &binop_sig)
            .unwrap();
        let build_slice_func = module
            .declare_function("jit_build_slice", Linkage::Import, &binop_sig)
            .unwrap();
        let store_subscr_func = module
            .declare_function("jit_store_subscr", Linkage::Import, &call_sig)
            .unwrap();
        let is_op_func = module
            .declare_function("jit_is_op", Linkage::Import, &call_sig)
            .unwrap();
        let invert_func = module
            .declare_function("jit_invert", Linkage::Import, &unary_sig)
            .unwrap();
        let import_name_func = module
            .declare_function("jit_import_name", Linkage::Import, &import_sig)
            .unwrap();
        let import_from_func = module
            .declare_function("jit_import_from", Linkage::Import, &import_from_sig)
            .unwrap();
        let unpack_ex_func = module
            .declare_function("jit_unpack_ex", Linkage::Import, &unpack_ex_sig)
            .unwrap();
        let setup_with_func = module
            .declare_function("jit_setup_with", Linkage::Import, &context_sig)
            .unwrap();
        let with_exit_func = module
            .declare_function("jit_with_exit", Linkage::Import, &context_sig)
            .unwrap();
        let make_function_func = module
            .declare_function("jit_make_function", Linkage::Import, &make_function_sig)
            .unwrap();
        JitCompiler {
            builder_context: FunctionBuilderContext::new(),
            module,
            add_func,
            sub_func,
            mul_func,
            div_func,
            floor_div_func,
            mod_func,
            pow_func,
            lshift_func,
            rshift_func,
            bit_and_func,
            bit_or_func,
            bit_xor_func,
            inplace_binop_func,
            getitem_func,
            cmp_func,
            truthy_func,
            neg_func,
            not_func,
            build_list_func,
            build_tuple_func,
            list_append_func,
            contains_func,
            get_iter_func,
            call_func,
            call_kw_func,
            load_attr_func,
            for_iter_func,
            build_map_func,
            store_attr_func,
            unpack_sequence_func,
            load_name_func,
            build_set_func,
            build_string_func,
            build_slice_func,
            store_subscr_func,
            is_op_func,
            invert_func,
            import_name_func,
            import_from_func,
            unpack_ex_func,
            setup_with_func,
            with_exit_func,
            make_function_func,
        }
    }

    fn make_binop_sig() -> cranelift::codegen::ir::Signature {
        let mut s =
            cranelift::codegen::ir::Signature::new(cranelift::codegen::isa::CallConv::SystemV);
        s.params.push(AbiParam::new(types::I64));
        s.params.push(AbiParam::new(types::I64));
        s.params.push(AbiParam::new(types::I64));
        s
    }

    fn make_cmp_sig() -> cranelift::codegen::ir::Signature {
        let mut s =
            cranelift::codegen::ir::Signature::new(cranelift::codegen::isa::CallConv::SystemV);
        s.params.push(AbiParam::new(types::I64));
        s.params.push(AbiParam::new(types::I64));
        s.params.push(AbiParam::new(types::I64));
        s.params.push(AbiParam::new(types::I64));
        s
    }

    fn make_call_kw_sig() -> cranelift::codegen::ir::Signature {
        let mut s =
            cranelift::codegen::ir::Signature::new(cranelift::codegen::isa::CallConv::SystemV);
        s.params.push(AbiParam::new(types::I64)); // func ptr
        s.params.push(AbiParam::new(types::I64)); // npos
        s.params.push(AbiParam::new(types::I64)); // nkw
        s.params.push(AbiParam::new(types::I64)); // items array ptr
        s.params.push(AbiParam::new(types::I64)); // out ptr
        s
    }

    fn make_truthy_sig() -> cranelift::codegen::ir::Signature {
        let mut s =
            cranelift::codegen::ir::Signature::new(cranelift::codegen::isa::CallConv::SystemV);
        s.params.push(AbiParam::new(types::I64));
        s.returns.push(AbiParam::new(types::I64));
        s
    }

    fn make_unary_sig() -> cranelift::codegen::ir::Signature {
        let mut s =
            cranelift::codegen::ir::Signature::new(cranelift::codegen::isa::CallConv::SystemV);
        s.params.push(AbiParam::new(types::I64));
        s.params.push(AbiParam::new(types::I64));
        s
    }

    fn make_call_sig() -> cranelift::codegen::ir::Signature {
        let mut s =
            cranelift::codegen::ir::Signature::new(cranelift::codegen::isa::CallConv::SystemV);
        s.params.push(AbiParam::new(types::I64)); // func ptr
        s.params.push(AbiParam::new(types::I64)); // nargs
        s.params.push(AbiParam::new(types::I64)); // args array ptr
        s.params.push(AbiParam::new(types::I64)); // out ptr
        s
    }

    fn make_load_attr_sig() -> cranelift::codegen::ir::Signature {
        let mut s =
            cranelift::codegen::ir::Signature::new(cranelift::codegen::isa::CallConv::SystemV);
        s.params.push(AbiParam::new(types::I64)); // obj ptr
        s.params.push(AbiParam::new(types::I64)); // names array ptr
        s.params.push(AbiParam::new(types::I64)); // name_idx
        s.params.push(AbiParam::new(types::I64)); // out ptr
        s
    }

    fn make_store_attr_sig() -> cranelift::codegen::ir::Signature {
        let mut s =
            cranelift::codegen::ir::Signature::new(cranelift::codegen::isa::CallConv::SystemV);
        s.params.push(AbiParam::new(types::I64));
        s.params.push(AbiParam::new(types::I64));
        s.params.push(AbiParam::new(types::I64));
        s.params.push(AbiParam::new(types::I64));
        s.params.push(AbiParam::new(types::I64));
        s
    }

    fn make_unpack_sig() -> cranelift::codegen::ir::Signature {
        let mut s =
            cranelift::codegen::ir::Signature::new(cranelift::codegen::isa::CallConv::SystemV);
        s.params.push(AbiParam::new(types::I64));
        s.params.push(AbiParam::new(types::I64));
        s.params.push(AbiParam::new(types::I64));
        s.params.push(AbiParam::new(types::I64));
        s.returns.push(AbiParam::new(types::I64));
        s
    }

    // make_import_sig: consts ptr, names_offset, name_idx, out ptr
    fn make_import_sig() -> cranelift::codegen::ir::Signature {
        let mut s =
            cranelift::codegen::ir::Signature::new(cranelift::codegen::isa::CallConv::SystemV);
        s.params.push(AbiParam::new(types::I64));
        s.params.push(AbiParam::new(types::I64));
        s.params.push(AbiParam::new(types::I64));
        s.params.push(AbiParam::new(types::I64));
        s
    }

    // make_import_from_sig: module ptr, consts ptr, names_offset, name_idx, out ptr
    fn make_import_from_sig() -> cranelift::codegen::ir::Signature {
        let mut s =
            cranelift::codegen::ir::Signature::new(cranelift::codegen::isa::CallConv::SystemV);
        s.params.push(AbiParam::new(types::I64));
        s.params.push(AbiParam::new(types::I64));
        s.params.push(AbiParam::new(types::I64));
        s.params.push(AbiParam::new(types::I64));
        s.params.push(AbiParam::new(types::I64));
        s
    }

    // make_unpack_ex_sig: seq ptr, n_before, n_after, items ptr, out ptr -> i64
    fn make_unpack_ex_sig() -> cranelift::codegen::ir::Signature {
        let mut s =
            cranelift::codegen::ir::Signature::new(cranelift::codegen::isa::CallConv::SystemV);
        s.params.push(AbiParam::new(types::I64));
        s.params.push(AbiParam::new(types::I64));
        s.params.push(AbiParam::new(types::I64));
        s.params.push(AbiParam::new(types::I64));
        s.params.push(AbiParam::new(types::I64));
        s.returns.push(AbiParam::new(types::I64));
        s
    }

    // make_context_sig: mgr ptr, out ptr
    fn make_make_function_sig() -> cranelift::codegen::ir::Signature {
        let mut s =
            cranelift::codegen::ir::Signature::new(cranelift::codegen::isa::CallConv::SystemV);
        s.params.push(AbiParam::new(types::I64));
        s.params.push(AbiParam::new(types::I64));
        s.params.push(AbiParam::new(types::I64));
        s.returns.push(AbiParam::new(types::I64));
        s
    }

    fn make_context_sig() -> cranelift::codegen::ir::Signature {
        let mut s =
            cranelift::codegen::ir::Signature::new(cranelift::codegen::isa::CallConv::SystemV);
        s.params.push(AbiParam::new(types::I64));
        s.params.push(AbiParam::new(types::I64));
        s
    }

    pub fn is_enabled() -> bool {
        true
    }

    pub fn precompute_consts(code: &CodeObject) -> Vec<PyObjectRef> {
        // Delegates to `vm::eval_const_value` — the interpreter's own,
        // already-correct textual-constant parser (handles `0x`/`0o`/`0b`
        // prefixes and `_` digit separators for `ConstValue::Int`, real
        // `Complex` construction, etc.) — rather than a second, ad hoc
        // copy. This file's own previous copy only ever tried plain
        // base-10 parsing, so ANY int literal written in hex/octal/binary
        // (`0xFFFF`, `0o17`, `0b101`) inside a JIT-eligible (loop-
        // containing) function panicked via `.unwrap()` on the fallback
        // BigInt parse — dormant until a loop happened to also contain
        // such a literal.
        code.consts
            .iter()
            .map(|cv| {
                crate::vm::eval_const_value(cv.clone()).unwrap_or_else(|_| crate::object::py_none())
            })
            .collect()
    }

    /// Precompute constants AND resolve globals for JIT.
    /// Returns [consts..., globals...] so LOAD_GLOBAL can index past consts.
    pub fn precompute_with_globals(
        code: &CodeObject,
        globals: &std::collections::HashMap<String, crate::object::PyObjectRef>,
        builtins: &std::collections::HashMap<String, crate::object::PyObjectRef>,
    ) -> Vec<crate::object::PyObjectRef> {
        let mut result = Self::precompute_consts(code);
        let base = result.len();
        result.resize(base + code.names.len(), crate::object::py_none());
        for (i, name) in code.names.iter().enumerate() {
            let name_str = crate::interner::lookup_str(*name);
            let val = globals
                .get(name_str)
                .or_else(|| builtins.get(name_str))
                .cloned()
                .unwrap_or_else(crate::object::py_none);
            result[base + i] = val;
        }
        result
    }

    pub fn precompute_with_names(code: &CodeObject) -> Vec<PyObjectRef> {
        let mut result = Self::precompute_consts(code);
        for name in &code.names {
            result.push(crate::object::py_str(crate::interner::lookup_str(*name)));
        }
        result
    }

    /// Precompute the JIT consts array with the layout
    /// `[consts (C), global-values (N), name-strings (N)]`:
    /// - LOAD_GLOBAL indexes past the consts into the global-VALUES region,
    /// - LOAD_ATTR / STORE_ATTR index past consts + globals into the
    ///   name-STRING region.
    /// (The previous `precompute_with_names` stored only the name STRINGS,
    /// so LOAD_GLOBAL pushed a name string instead of the resolved value —
    /// every JIT-compiled loop that touched a global got the wrong object.)
    pub fn precompute_for_jit(
        code: &CodeObject,
        globals: &Rc<RefCell<HashMap<crate::interner::StrId, crate::object::PyObjectRef>>>,
        builtins: &HashMap<crate::interner::StrId, crate::object::PyObjectRef>,
    ) -> Vec<crate::object::PyObjectRef> {
        let mut result = Self::precompute_consts(code);
        let c = result.len();
        let n = code.names.len();
        result.resize(c + 2 * n, crate::object::py_none());
        let g = globals.borrow();
        for (i, name) in code.names.iter().enumerate() {
            let name_str = crate::interner::lookup_str(*name);
            let val = g
                .get(name)
                .cloned()
                .or_else(|| builtins.get(name).cloned())
                .unwrap_or_else(crate::object::py_none);
            result[c + i] = val;
            result[c + n + i] = crate::object::py_str(name_str);
        }
        result
    }

}