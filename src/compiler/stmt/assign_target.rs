use crate::ast::*;
use crate::bytecode::*;
use super::super::Compiler;
use super::super::scope::ScopeType;

impl Compiler {
    pub(crate) fn compile_assign_target(&mut self, target: &Expr) -> Result<(), String> {
        match target {
            Expr::Name(name) => {
                // Mirrors the read-side (`compile_expr`'s own `Expr::Name`
                // arm) mangling fix — a bare private-name ASSIGNMENT
                // target within a class body must resolve to the SAME
                // mangled storage key any later bare-name READ of it will
                // look up.
                let name = &self.mangle_name(name);
                if self.scope == ScopeType::Module
                    || self.scope == ScopeType::ClassBody
                    || self.global_names.contains(name)
                {
                    let idx = self.get_name_index(name) as u32;
                    self.emit(Opcode::STORE_NAME, idx);
                } else if self.scope == ScopeType::Function && self.code.cellvars.contains(name) {
                    // Cell variable: use STORE_DEREF
                    let idx = self.code.cellvars.iter().position(|n| n == name).unwrap() as u32;
                    self.emit(Opcode::STORE_DEREF, idx);
                } else if self.scope == ScopeType::Function && self.code.freevars.contains(name) {
                    // Free variable (nonlocal): use STORE_DEREF
                    let fv_idx = self.code.freevars.iter().position(|n| n == name).unwrap();
                    let idx = (self.code.cellvars.len() + fv_idx) as u32;
                    self.emit(Opcode::STORE_DEREF, idx);
                } else {
                    let idx = self.add_varname(name) as u32;
                    self.emit(Opcode::STORE_FAST, idx);
                }
            }
            Expr::Attribute { value, attr } => {
                self.compile_expr(value)?;
                // Stack is [..., val, obj] — swap to [..., obj, val] for STORE_ATTR
                self.emit(Opcode::SWAP, 1);
                let idx = self.get_name_index(&self.mangle_name(attr)) as u32;
                self.emit(Opcode::STORE_ATTR, idx);
            }
            Expr::Subscript { value, slice } => {
                // Entered with the value-to-assign already on the stack
                // (chained assignment's COPY, or a tuple/list-unpack
                // target's per-element value) — stack is [value, obj, slice]
                // after pushing obj/slice below, but STORE_SUBSCR needs
                // [obj, slice, value] (matching the single-target
                // `Stmt::Assign` case above, which pushes obj/slice BEFORE
                // the value for exactly this reason). Reorder with two
                // SWAPs instead of restructuring the value-already-pushed
                // calling convention every caller of compile_assign_target
                // relies on.
                self.compile_expr(value)?;
                self.compile_expr(slice)?;
                self.emit(Opcode::SWAP, 1);
                self.emit(Opcode::SWAP, 2);
                self.emit(Opcode::STORE_SUBSCR, 0);
            }
            Expr::Starred(inner) => {
                // Starred target: unwrap and compile inner target
                self.compile_assign_target(inner)?;
            }
            Expr::List(elts) | Expr::Tuple(elts) => {
                // Check if any element is a Starred target — use UNPACK_EX if so
                let star_pos = elts.iter().position(|e| matches!(e, Expr::Starred(_)));
                if let Some(pos) = star_pos {
                    let before = pos;
                    let after = elts.len() - pos - 1;
                    let arg = ((before as u32) << 8) | (after as u32);
                    self.emit(Opcode::UNPACK_EX, arg);
                    for elt in elts {
                        self.compile_assign_target(elt)?;
                    }
                } else {
                    let count = elts.len();
                    self.emit(Opcode::UNPACK_SEQUENCE, count as u32);
                    for elt in elts {
                        self.compile_assign_target(elt)?;
                    }
                }
            }
            _ => {
                // Real CPython's exact wording is "cannot assign to X" per
                // target kind (dict/list/set comprehension, generator
                // expression, function call, ...) — lowercase `cannot`,
                // and never the Rust `Debug` dump of the AST node this
                // used to produce (`Cannot assign to DictComp { key: ... }`,
                // which also failed `assertRaisesRegex(SyntaxError,
                // "cannot assign")` purely on the capital `C`). Named the
                // common comprehension/generator cases explicitly since
                // those are what CPython's own test suite exercises
                // (`test_dictcomps.py`/`test_listcomps.py`/`test_setcomps.py`
                // all assign to a comprehension to check this exact error).
                let what = match target {
                    Expr::DictComp { .. } => "dict comprehension".to_string(),
                    Expr::ListComp { .. } => "list comprehension".to_string(),
                    Expr::SetComp { .. } => "set comprehension".to_string(),
                    Expr::GeneratorExp { .. } => "generator expression".to_string(),
                    Expr::Call { .. } => "function call".to_string(),
                    Expr::Constant(_) => "literal".to_string(),
                    Expr::Compare { .. } => "comparison".to_string(),
                    Expr::BinOp { .. } => "operator".to_string(),
                    Expr::BoolOp { .. } => "operator".to_string(),
                    Expr::UnaryOp { .. } => "operator".to_string(),
                    Expr::Lambda { .. } => "lambda".to_string(),
                    _ => "expression".to_string(),
                };
                return Err(format!("cannot assign to {}", what));
            }
        }
        Ok(())
    }

}
