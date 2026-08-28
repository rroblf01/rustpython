use crate::bytecode::Opcode;
use crate::object::*;
use crate::vm::VirtualMachine;

impl VirtualMachine {
    pub(crate) fn handle_stack(&mut self, fi: usize, op: Opcode, arg: u32) -> PyResult<()> {
        match op {
            Opcode::POP_TOP => {
                self.frames[fi].pop()?;
            }

            Opcode::DUP_TOP => {
                let val = self.frames[fi].peek(0)?;
                self.frames[fi].push(val);
            }

            Opcode::COPY => {
                let depth = arg as usize;
                if depth >= self.frames[fi].stack.len() {
                    if let Some(val) = self.frames[fi].stack.last().cloned() {
                        self.frames[fi].push(val);
                    } else {
                        return Err(PyError::runtime_error("stack underflow (peek)"));
                    }
                } else {
                    let val = self.frames[fi].peek(depth)?;
                    self.frames[fi].push(val);
                }
            }

            Opcode::SWAP => {
                let i = arg as usize;
                let len = self.frames[fi].stack.len();
                if i > 0 && i < len {
                    self.frames[fi].stack.swap(len - 1, len - 1 - i);
                }
            }

            Opcode::GET_LEN => {
                let obj = self.frames[fi].pop()?;
                let len = crate::object::builtin_len(&[obj])?;
                self.frames[fi].push(len);
            }
            Opcode::MATCH_MAPPING => {
                let subject = self.frames[fi].peek(0)?;
                let is_map = matches!(
                    &*subject.borrow(),
                    PyObject::Dict(_) | PyObject::Instance { .. }
                );
                self.frames[fi].push(py_bool(is_map));
            }
            Opcode::MATCH_SEQUENCE => {
                let subject = self.frames[fi].peek(0)?;
                let is_seq = matches!(
                    &*subject.borrow(),
                    PyObject::List(_)
                        | PyObject::Tuple(_)
                        | PyObject::Str(_)
                        | PyObject::Bytes(_)
                        | PyObject::ByteArray(_)
                );
                self.frames[fi].push(py_bool(is_seq));
            }
            Opcode::MATCH_KEYS => {
                let _keys = self.frames[fi].pop()?;
                self.frames[fi].push(py_bool(true));
            }
            Opcode::CALL_INTRINSIC_1 => {
                let intrinsic = arg;
                match intrinsic {
                    1 => {
                        self.frames[fi].push(py_int(0));
                    }
                    2 => {
                        let val = self.frames[fi].pop()?;
                        let _ = print!("{}", val.str());
                        self.frames[fi].push(py_none());
                    }
                    _ => {
                        self.frames[fi].push(py_none());
                    }
                }
            }
            Opcode::CALL_INTRINSIC_2 => {
                self.frames[fi].push(py_int(0));
            }
            Opcode::UNPACK_SEQUENCE_TWO_TUPLE => {
                let seq = self.frames[fi].pop()?;
                let seq_borrowed = seq.borrow();
                if let PyObject::Tuple(items) = &*seq_borrowed {
                    if items.len() >= 2 {
                        self.frames[fi].push(items[0].clone());
                        self.frames[fi].push(items[1].clone());
                    } else {
                        return Err(PyError::runtime_error("not enough values to unpack"));
                    }
                } else if let PyObject::List(items) = &*seq_borrowed {
                    if items.len() >= 2 {
                        self.frames[fi].push(items[0].clone());
                        self.frames[fi].push(items[1].clone());
                    } else {
                        return Err(PyError::runtime_error("not enough values to unpack"));
                    }
                } else {
                    let it = crate::object::builtin_iter(&[seq.clone()])?;
                    let v1 = crate::object::builtin_next(&[it.clone()])?;
                    let v2 = crate::object::builtin_next(&[it.clone()])?;
                    self.frames[fi].push(v1);
                    self.frames[fi].push(v2);
                }
            }

            _ => {}
        }
        Ok(())
    }
}
