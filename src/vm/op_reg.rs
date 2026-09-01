use crate::bytecode::{ConstValue, Opcode};
use crate::interner;
use crate::object::*;
use crate::vm::VirtualMachine;
use num_bigint::BigInt;
use std::rc::Rc;

impl VirtualMachine {
    pub(crate) fn handle_reg(&mut self, fi: usize, op: Opcode, arg: u32) -> PyResult<Option<PyObjectRef>> {
        match op {
            Opcode::REG_MOV => {
                // Lazily initialize registers
                if self.frames[fi].registers.is_empty() {
                    self.frames[fi].registers = Box::new(vec![None; 256]);
                }
                let dst = (arg >> 4) as usize;
                let src = (arg & 0xF) as usize;
                let val = self.frames[fi].registers[src]
                    .clone()
                    .ok_or_else(|| PyError::runtime_error("REG_MOV: source register is empty"))?;
                if dst < self.frames[fi].registers.len() {
                    self.frames[fi].registers[dst] = Some(val);
                }
                Ok(None)
            }
            Opcode::REG_LOAD_CONST => {
                let dst = (arg >> 4) as usize;
                let const_idx = (arg & 0xFF) as usize;
                let const_val = self.frames[fi]
                    .code
                    .consts
                    .get(const_idx)
                    .ok_or_else(|| PyError::runtime_error("REG_LOAD_CONST: index out of range"))?
                    .clone();
                let obj = match const_val {
                    ConstValue::None => py_none(),
                    ConstValue::Bool(b) => py_bool(b),
                    ConstValue::Int(s) => {
                        if let Ok(n) = s.parse::<i64>() {
                            py_int(n)
                        } else {
                            let n: BigInt =
                                s.parse().map_err(|_| PyError::value_error("invalid int"))?;
                            PyObjectRef::imm(PyObject::Int(n))
                        }
                    }
                    ConstValue::Float(s) => py_float(
                        s.parse()
                            .map_err(|_| PyError::value_error("invalid float"))?,
                    ),
                    ConstValue::String(s) => py_str(&s),
                    ConstValue::Bytes(b) => PyObjectRef::imm(PyObject::Bytes(b)),
                    ConstValue::Complex { real, imag } => {
                        let re: f64 = real
                            .parse()
                            .map_err(|_| PyError::value_error("invalid complex literal"))?;
                        let im: f64 = imag
                            .parse()
                            .map_err(|_| PyError::value_error("invalid complex literal"))?;
                        PyObjectRef::imm(PyObject::Complex(re, im))
                    }
                    ConstValue::Code(code) => PyObjectRef::imm(PyObject::Code(Rc::from(code))),
                    ConstValue::Tuple(items) => {
                        let objs: Vec<PyObjectRef> =
                            items.into_iter().map(|s| py_str(&s)).collect();
                        PyObjectRef::imm(PyObject::Tuple(objs))
                    }
                };
                if dst < self.frames[fi].registers.len() {
                    self.frames[fi].registers[dst] = Some(obj);
                }
                Ok(None)
            }
            Opcode::REG_LOAD_FAST => {
                let dst = (arg >> 4) as usize;
                let var_idx = (arg & 0xFF) as usize;
                let val = self.frames[fi].fast_locals.get(var_idx).and_then(|v| v.clone())
                    .ok_or_else(|| PyError::unbound_local_error(format!("cannot access local variable '{}' where it is not associated with a value",
                        self.frames[fi].code.varnames.get(var_idx).map_or("?", |&s| crate::interner::lookup_str(s)))))?;
                if dst < self.frames[fi].registers.len() {
                    self.frames[fi].registers[dst] = Some(val);
                }
                Ok(None)
            }
            Opcode::REG_STORE_FAST => {
                let src = (arg >> 4) as usize;
                let var_idx = (arg & 0xFF) as usize;
                let val = self.frames[fi].registers[src].clone().ok_or_else(|| {
                    PyError::runtime_error("REG_STORE_FAST: source register is empty")
                })?;
                if var_idx < self.frames[fi].fast_locals.len() {
                    self.frames[fi].fast_locals[var_idx] = Some(val.clone());
                }
                let name = Some(crate::interner::lookup_str(
                    self.frames[fi].code.varnames[var_idx],
                ))
                .ok_or_else(|| PyError::runtime_error("varname index out of range"))?
                .clone();
                self.frames[fi].insert_local(&name, val);
                Ok(None)
            }
            Opcode::REG_BINARY_OP => {
                let dst = (arg >> 4) as usize;
                let a_reg = ((arg >> 2) & 0x3) as usize;
                let b_reg = (arg & 0x3) as usize;
                let op = (arg >> 8) as u32;
                let a = self.frames[fi].registers[a_reg]
                    .clone()
                    .ok_or_else(|| PyError::runtime_error("REG_BINARY_OP: a is empty"))?;
                let b = self.frames[fi].registers[b_reg]
                    .clone()
                    .ok_or_else(|| PyError::runtime_error("REG_BINARY_OP: b is empty"))?;
                let result = match op {
                    0 => py_add(&a, &b),
                    1 => py_sub(&a, &b),
                    2 => py_mul(&a, &b),
                    3 => py_div(&a, &b),
                    4 => py_floor_div(&a, &b),
                    5 => py_mod(&a, &b),
                    6 => py_pow(&a, &b),
                    7 => py_lshift(&a, &b),
                    8 => py_rshift(&a, &b),
                    9 => py_bit_or(&a, &b),
                    10 => py_bit_xor(&a, &b),
                    11 => py_bit_and(&a, &b),
                    13 => py_getitem(&a, &b),
                    _ => {
                        return Err(PyError::runtime_error(format!(
                            "unknown reg binary op: {}",
                            op
                        )))
                    }
                }?;
                if dst < self.frames[fi].registers.len() {
                    self.frames[fi].registers[dst] = Some(result);
                }
                Ok(None)
            }
            Opcode::REG_LOAD_GLOBAL => {
                let dst = (arg >> 4) as usize;
                let name_idx = (arg & 0xFF) as usize;
                let name = crate::interner::lookup_str(self.frames[fi].code.names[name_idx]);
                // Check inline cache first
                let instr_ip = self.frames[fi].ip - 1;
                if let Some(cached) = self.frames[fi]
                    .global_cache
                    .get(instr_ip)
                    .and_then(|c| c.clone())
                {
                    if dst < self.frames[fi].registers.len() {
                        self.frames[fi].registers[dst] = Some(cached);
                    }
                } else {
                    let val = self.frames[fi]
                        .globals
                        .borrow()
                        .get(&interner::intern(name))
                        .cloned()
                        .or_else(|| {
                            self.frames[fi]
                                .builtins
                                .get(&interner::intern(name))
                                .cloned()
                        });
                    if let Some(v) = val {
                        if instr_ip < self.frames[fi].global_cache.len() {
                            self.frames[fi].global_cache[instr_ip] = Some(v.clone());
                        }
                        if dst < self.frames[fi].registers.len() {
                            self.frames[fi].registers[dst] = Some(v);
                        }
                    } else {
                        return Err(PyError::name_error(format!(
                            "name '{}' is not defined",
                            name
                        )));
                    }
                }
                Ok(None)
            }
            Opcode::REG_RETURN => {
                let src = (arg & 0xFF) as usize;
                let val = self.frames[fi].registers[src]
                    .clone()
                    .ok_or_else(|| PyError::runtime_error("REG_RETURN: register is empty"))?;
                return Ok(Some(val));
            }
            Opcode::REG_BUILD_LIST => {
                // arg: upper 4 bits = dst, lower 4 bits = count
                let dst = (arg >> 4) as usize;
                let count = (arg & 0xF) as usize;
                let mut items = Vec::with_capacity(count);
                for i in 0..count {
                    if let Some(val) = self.frames[fi].registers[i].clone() {
                        items.push(val);
                    }
                }
                if dst < self.frames[fi].registers.len() {
                    self.frames[fi].registers[dst] = Some(py_list(items));
                }
                Ok(None)
            }
            _ => Ok(None),
        }
    }
}
