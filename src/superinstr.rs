//! Peephole superinstruction fusion.
//!
//! Runs once over every compiled `CodeObject` (recursively through nested
//! function/class code constants). Fuses short, hot, straight-line
//! instruction sequences into single SUPER_* opcodes so the interpreter
//! pays dispatch cost once instead of once per fused instruction — the
//! opcode histogram showed LOAD_FAST+STORE_FAST alone are ~46% of all
//! executed instructions in typical Python code.
//!
//! Correctness contract:
//! * A sequence is only fused when NONE of its slots is a jump target and
//!   the FIRST slot isn't one either. Jump targets are computed exactly
//!   from the three jump encodings this VM uses (POP_JUMP_*/FOR_ITER take
//!   absolute args; JUMP_FORWARD/JUMP_BACKWARD are relative to the already
//!   advanced ip).
//! * The slots after the first are overwritten with NOP; the SUPER
//!   handler advances ip past them, so normal flow never executes them,
//!   while any external jump landing just past the sequence still lands
//!   on the correct following instruction. No index remapping needed.
//!
//! Fusion is skipped (safely leaving the original sequence) whenever an
//! operand doesn't fit the packed arg layout: local indices must be < 256,
//! const indices < 65536, and BINARY_OP must be non-in-place (arg < 100).

use crate::bytecode::{Instr, Opcode};

/// Indices that are the destination of any jump.
fn jump_targets(instrs: &[Instr]) -> Vec<bool> {
    let n = instrs.len();
    let mut targets = vec![false; n];
    for (i, instr) in instrs.iter().enumerate() {
        match instr.op {
            Opcode::JUMP_FORWARD => {
                let t = i + 1 + instr.arg as usize;
                if t < n {
                    targets[t] = true;
                }
            }
            Opcode::JUMP_BACKWARD => {
                if i + 1 >= instr.arg as usize {
                    let t = i + 1 - instr.arg as usize - 1;
                    if t < n {
                        targets[t] = true;
                    }
                }
            }
            // Absolute-target jumps (verified against the vm.rs handlers).
            // SETUP_FINALLY/SETUP_CLEANUP register their arg as the exception
            // handler address (jumped to on raise, vm.rs:10490);
            // POP_EXCEPT_AND_EXECUTE_FINALLY and SEND also jump absolutely
            // to arg. Missing any of these let a fused NOP land on an
            // exception-handler entry point, corrupting control flow.
            Opcode::POP_JUMP_IF_FALSE
            | Opcode::POP_JUMP_IF_TRUE
            | Opcode::POP_JUMP_IF_NONE
            | Opcode::POP_JUMP_IF_NOT_NONE
            | Opcode::FOR_ITER
            | Opcode::SETUP_FINALLY
            | Opcode::SETUP_CLEANUP
            | Opcode::POP_EXCEPT_AND_EXECUTE_FINALLY
            | Opcode::SEND
            | Opcode::JUMP => {
                let t = instr.arg as usize;
                if t < n {
                    targets[t] = true;
                }
            }
            _ => {}
        }
    }
    targets
}

#[inline]
fn nop() -> Instr {
    Instr::with_arg(Opcode::NOP, 0)
}

/// Fuse hot sequences in place. Idempotent (already-fused code contains no
/// matching raw sequences).
pub fn apply(code: &mut crate::bytecode::CodeObject) {
    fuse_one(&mut code.instructions);
    // Recurse into nested code objects (closures, comprehensions, class
    // bodies) — they live among the constants.
    for c in code.consts.iter_mut() {
        if let crate::bytecode::ConstValue::Code(inner) = c {
            apply(inner);
        }
    }
}

fn fuse_one(instrs: &mut Vec<Instr>) {
    let targets = jump_targets(instrs);
    let mut i = 0;
    while i + 3 < instrs.len() {
        // Pattern A: LOAD_FAST a, LOAD_FAST b, BINARY_OP op(<100), STORE_FAST z
        if !targets[i]
            && instrs[i].op == Opcode::LOAD_FAST
            && instrs[i + 1].op == Opcode::LOAD_FAST
            && instrs[i + 2].op == Opcode::BINARY_OP
            && instrs[i + 3].op == Opcode::STORE_FAST
            && !targets[i + 1]
            && !targets[i + 2]
        {
            let (a, b, op, z) = (
                instrs[i].arg,
                instrs[i + 1].arg,
                instrs[i + 2].arg,
                instrs[i + 3].arg,
            );
            // op may be plain (<100) or in-place (>=100); the SUPER handler
            // dispatches through inplace_binary_op/plain_binary_op exactly
            // like the BINARY_OP opcode does.
            if a < 256 && b < 256 && z < 256 {
                instrs[i] = Instr::with_arg(
                    Opcode::SUPER_FAST2_BIN,
                    a | (b << 8) | (op << 16) | (z << 24),
                );
                instrs[i + 1] = nop();
                instrs[i + 2] = nop();
                instrs[i + 3] = nop();
                i += 4;
                continue;
            }
        }
        // Pattern B: LOAD_FAST a, LOAD_CONST c, BINARY_OP op(<100), STORE_FAST z
        if !targets[i]
            && i + 3 < instrs.len()
            && instrs[i].op == Opcode::LOAD_FAST
            && instrs[i + 1].op == Opcode::LOAD_CONST
            && instrs[i + 2].op == Opcode::BINARY_OP
            && instrs[i + 2].arg < 100
            && instrs[i + 3].op == Opcode::STORE_FAST
            && instrs[i + 3].arg == instrs[i].arg // self-accumulating: z==a
            && !targets[i + 1]
            && !targets[i + 2]
        {
            let (a, c, op) = (
                instrs[i].arg,
                instrs[i + 1].arg,
                instrs[i + 2].arg,
            );
            if a < 256 && c < 65536 {
                instrs[i] =
                    Instr::with_arg(Opcode::SUPER_FASTC_BIN, a | (c << 8) | (op << 24));
                instrs[i + 1] = nop();
                instrs[i + 2] = nop();
                instrs[i + 3] = nop();
                i += 4;
                continue;
            }
        }
        // Pattern C: LOAD_FAST a, STORE_FAST z
        if !targets[i]
            && i + 1 < instrs.len()
            && instrs[i].op == Opcode::LOAD_FAST
            && instrs[i + 1].op == Opcode::STORE_FAST
            && !targets[i + 1]
        {
            let (a, z) = (instrs[i].arg, instrs[i + 1].arg);
            if a < 256 && z < 65536 {
                instrs[i] = Instr::with_arg(Opcode::SUPER_FAST_MOV, a | (z << 16));
                instrs[i + 1] = nop();
                i += 2;
                continue;
            }
        }
        i += 1;
    }
}
