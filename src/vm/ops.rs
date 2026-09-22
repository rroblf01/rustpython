use crate::bytecode::Opcode;
use crate::object::*;
use crate::vm::VirtualMachine;

impl VirtualMachine {
    /// Handles BUILD_*, BINARY_OP, COMPARE, UNARY and JUMP opcodes.
    /// Returns Ok(true) if `op` was handled here, Ok(false) if not.
    pub(crate) fn handle_build_arith_control(
        &mut self,
        fi: usize,
        op: Opcode,
        arg: u32,
    ) -> PyResult<bool> {
        match op {
            Opcode::BUILD_LIST => {
                let count = arg as usize;
                let mut items = Vec::with_capacity(count);
                for _ in 0..count {
                    items.push(self.frames[fi].pop()?);
                }
                items.reverse();
                self.frames[fi].push(py_list(items));
            }
            Opcode::BUILD_TUPLE => {
                let count = arg as usize;
                let mut items = Vec::with_capacity(count);
                for _ in 0..count {
                    items.push(self.frames[fi].pop()?);
                }
                items.reverse();
                self.frames[fi].push(py_tuple(items));
            }
            Opcode::BUILD_MAP => {
                self.frames[fi].push(py_dict());
            }
            Opcode::BUILD_SET => {
                let count = arg as usize;
                let mut items = Vec::with_capacity(count);
                for _ in 0..count {
                    items.push(self.frames[fi].pop()?);
                }
                items.reverse();
                self.frames[fi].push(PyObjectRef::new(PyObject::Set(PySet::from_vec(items)?)));
            }
            Opcode::BUILD_STRING => {
                let count = arg as usize;
                let mut parts = Vec::with_capacity(count);
                for _ in 0..count {
                    parts.push(self.frames[fi].pop()?.str());
                }
                parts.reverse();
                self.frames[fi].push(py_str(&parts.join("")));
            }
            // PEP 750: pop the interpolations tuple, then the strings
            // tuple, and build a `Template`.
            Opcode::BUILD_TEMPLATE => {
                let interpolations = self.frames[fi].pop()?;
                let strings = self.frames[fi].pop()?;
                let strings_vec = match &*strings.borrow() {
                    PyObject::Tuple(v) => v.clone(),
                    _ => {
                        return Err(PyError::type_error(
                            "BUILD_TEMPLATE expects a strings tuple".to_string(),
                        ))
                    }
                };
                let interpolations_vec = match &*interpolations.borrow() {
                    PyObject::Tuple(v) => v.clone(),
                    _ => {
                        return Err(PyError::type_error(
                            "BUILD_TEMPLATE expects an interpolations tuple".to_string(),
                        ))
                    }
                };
                let template = crate::object::build_template(strings_vec, interpolations_vec)?;
                self.frames[fi].push(template);
            }
            // PEP 750: stack is [value, expression, (format_spec?)] where
            // oparg bit 0 = has format_spec and bits >> 2 = conversion
            // (0 none, 1 !r, 2 !s, 3 !a — our internal codes).
            Opcode::BUILD_INTERPOLATION => {
                let has_spec = (arg & 1) != 0;
                let conversion =
                    match arg >> 2 {
                        0 => None,
                        1 => Some('r'),
                        2 => Some('s'),
                        3 => Some('a'),
                        _ => return Err(PyError::system_error(
                            "Interpolation() argument 'conversion' must be one of 's', 'a' or 'r'"
                                .to_string(),
                        )),
                    };
                let format_spec = if has_spec {
                    self.frames[fi].pop()?
                } else {
                    py_str("")
                };
                let expression = self.frames[fi].pop()?.str();
                let value = self.frames[fi].pop()?;
                let interpolation =
                    crate::object::build_interpolation(value, expression, conversion, format_spec)?;
                self.frames[fi].push(interpolation);
            }
            Opcode::BUILD_SLICE => {
                let nargs = arg as usize;
                let step = if nargs >= 3 {
                    Some(self.frames[fi].pop()?)
                } else {
                    None
                };
                let stop = if nargs >= 2 {
                    Some(self.frames[fi].pop()?)
                } else {
                    None
                };
                let start = if nargs >= 1 {
                    Some(self.frames[fi].pop()?)
                } else {
                    None
                };
                self.frames[fi].push(PyObjectRef::imm(PyObject::Slice {
                    start: start.unwrap_or(py_none()),
                    stop: stop.unwrap_or(py_none()),
                    step: step.unwrap_or(py_none()),
                }));
            }
            Opcode::BINARY_OP => {
                let (op_u, in_place) = if arg >= 100 {
                    (arg - 100, true)
                } else {
                    (arg, false)
                };
                let right = self.frames[fi].pop()?;
                let left = self.frames[fi].pop()?;
                let result = if in_place {
                    match crate::vm::helpers::inplace_binary_op(&left, &right, op_u)? {
                        Some(v) => v,
                        None => crate::vm::helpers::plain_binary_op(&left, &right, op_u)?,
                    }
                } else {
                    crate::vm::helpers::plain_binary_op(&left, &right, op_u)?
                };
                self.frames[fi].push(result);
            }
            Opcode::SUPER_FAST2_BIN => {
                let a = (arg & 0xFF) as usize;
                let b = ((arg >> 8) & 0xFF) as usize;
                let op_u = (arg >> 16) & 0xFF;
                let z = (arg >> 24) as usize;
                let (left, right) = {
                    let f = &self.frames[fi];
                    let right = f
                        .fast_locals
                        .get(b)
                        .cloned()
                        .flatten()
                        .ok_or_else(|| crate::vm::helpers::unbound_local_msg(f, b))?;
                    let left = f
                        .fast_locals
                        .get(a)
                        .cloned()
                        .flatten()
                        .ok_or_else(|| crate::vm::helpers::unbound_local_msg(f, a))?;
                    (left, right)
                };
                // Same-slot string accumulation (`s = s + t` with z == a, or
                // the `+=` form): CPython's `_BINARY_OP_INPLACE_ADD_UNICODE`
                // grows the string in place only for this exact pattern —
                // never for a bare refcount check (see `try_inplace_str_grow`).
                // NOTE: the slot is only BORROWED here — cloning it for the
                // identity check would add a third strong reference and
                // defeat the count==2 gate (CPython checks with borrowed
                // references for the same reason).
                let result = if (op_u == 0 || op_u == 100) && z == a {
                    let grown = {
                        let f = &self.frames[fi];
                        match f.fast_locals.get(a).and_then(|s| s.as_ref()) {
                            Some(slot_val) => {
                                crate::object::try_inplace_str_grow(slot_val, &left, &right)?
                            }
                            None => None,
                        }
                    };
                    match grown {
                        Some(v) => v,
                        None => crate::vm::helpers::plain_or_inplace(&left, &right, op_u)?,
                    }
                } else {
                    crate::vm::helpers::plain_or_inplace(&left, &right, op_u)?
                };
                let f = &mut self.frames[fi];
                if z < f.fast_locals.len() {
                    f.fast_locals[z] = Some(result);
                }
                f.ip += 3;
            }
            Opcode::SUPER_FASTC_BIN => {
                let a = (arg & 0xFF) as usize;
                let c = ((arg >> 8) & 0xFFFF) as usize;
                let op_u = (arg >> 24) as u32;
                let left = {
                    let f = &self.frames[fi];
                    f.fast_locals.get(a).cloned().flatten().ok_or_else(|| PyError::unbound_local_error(format!(
                        "cannot access local variable '{}' where it is not associated with a value",
                        crate::interner::lookup_str(f.code.varnames.get(a).copied().unwrap_or(crate::interner::intern("?")))
                    )))?
                };
                let cval = self.frames[fi]
                    .code
                    .consts
                    .get(c)
                    .cloned()
                    .ok_or_else(|| PyError::runtime_error("bad const index"))?;
                let right = crate::vm::helpers::eval_const_value(cval)?;
                // The fuser only emits this for `a = a op const` (same-slot
                // store, `superinstr.rs` Pattern B) — the exact pattern where
                // CPython's `_BINARY_OP_INPLACE_ADD_UNICODE` grows strings in
                // place. Anything else falls back to a fresh copy.
                // (Slot borrowed, not cloned — see the note above.)
                let result = if op_u == 0 {
                    let grown = {
                        let f = &self.frames[fi];
                        match f.fast_locals.get(a).and_then(|s| s.as_ref()) {
                            Some(slot_val) => {
                                crate::object::try_inplace_str_grow(slot_val, &left, &right)?
                            }
                            None => None,
                        }
                    };
                    match grown {
                        Some(v) => v,
                        None => crate::vm::helpers::plain_binary_op(&left, &right, op_u)?,
                    }
                } else {
                    crate::vm::helpers::plain_binary_op(&left, &right, op_u)?
                };
                let f = &mut self.frames[fi];
                if a < f.fast_locals.len() {
                    f.fast_locals[a] = Some(result);
                }
                f.ip += 3;
            }
            Opcode::SUPER_FAST_MOV => {
                let a = (arg & 0xFFFF) as usize;
                let z = (arg >> 16) as usize;
                let f = &mut self.frames[fi];
                let val = f
                    .fast_locals
                    .get(a)
                    .cloned()
                    .flatten()
                    .ok_or_else(|| crate::vm::helpers::unbound_local_msg(f, a))?;
                if z < f.fast_locals.len() {
                    f.fast_locals[z] = Some(val);
                }
                f.ip += 1;
            }
            Opcode::COMPARE_OP => {
                let op_u = arg;
                let right = self.frames[fi].pop()?;
                let left = self.frames[fi].pop()?;
                let result = py_compare(&left, &right, op_u)?;
                self.frames[fi].push(result);
            }
            Opcode::IS_OP => {
                let invert = arg != 0;
                let right = self.frames[fi].pop()?;
                let left = self.frames[fi].pop()?;
                let is_same = left.is(&right);
                let result = if invert { !is_same } else { is_same };
                self.frames[fi].push(py_bool(result));
            }
            Opcode::CONTAINS_OP => {
                let invert = arg != 0;
                let right = self.frames[fi].pop()?;
                let left = self.frames[fi].pop()?;
                // Operand order matches the JIT lowering (which pops
                // into (second, top) and calls the contains runtime the
                // same way) and what match-statement codegen emits:
                // the container is pushed first, the item second, so
                // `left` is the container here. (This used to pass them
                // swapped, which made every mapping-pattern key check
                // test `subject in key` instead of `key in subject` —
                // real trigger: `case {"key": x}` raised
                // `TypeError: 'in <string>' requires string as left
                // operand` for any dict subject.)
                let result = contains_op(&left, &right)?;
                let result = if invert { !result } else { result };
                self.frames[fi].push(py_bool(result));
            }
            Opcode::UNARY_NEGATIVE => {
                let val = self.frames[fi].pop()?;
                let neg_method = if let PyObject::Instance { typ, .. } = &*val.borrow() {
                    crate::object::lookup_dunder_via_mro(typ, "__neg__")
                } else {
                    None
                };
                let result = if let Some(f) = neg_method {
                    call_bound_method(f, val.clone(), vec![])?
                } else {
                    py_neg(&val)?
                };
                self.frames[fi].push(result);
            }
            Opcode::UNARY_POSITIVE => {
                let val = self.frames[fi].pop()?;
                let pos_method = if let PyObject::Instance { typ, .. } = &*val.borrow() {
                    crate::object::lookup_dunder_via_mro(typ, "__pos__")
                } else {
                    None
                };
                let result = if let Some(f) = pos_method {
                    call_bound_method(f, val.clone(), vec![])?
                } else {
                    py_pos(&val)?
                };
                self.frames[fi].push(result);
            }
            Opcode::UNARY_NOT => {
                let val = self.frames[fi].pop()?;
                self.frames[fi].push(py_bool(!val.try_truthy()?));
            }
            Opcode::UNARY_INVERT => {
                let val = self.frames[fi].pop()?;
                // A user-defined class (e.g. `enum.Flag`'s `__invert__`,
                // needed for `~SomeFlag.MEMBER`) overriding `__invert__` was
                // never consulted here — every OTHER unary operator
                // (`UNARY_NEGATIVE`/`UNARY_POSITIVE` just above) already
                // checks `lookup_dunder_via_mro` on `PyObject::Instance`
                // before falling back to the native-value cases; `~` alone
                // skipped straight to the native match and raised a bogus
                // `TypeError: bad operand type for unary ~` for ANY
                // Instance, even one that legitimately implements
                // `__invert__`.
                let invert_method = if let PyObject::Instance { typ, .. } = &*val.borrow() {
                    crate::object::lookup_dunder_via_mro(typ, "__invert__")
                } else {
                    None
                };
                let result = if let Some(f) = invert_method {
                    call_bound_method(f, val.clone(), vec![])?
                } else {
                    let obj = val.borrow();
                    match &*obj {
                        PyObject::Int(i) => py_int(!i),
                        PyObject::Bool(b) => {
                            crate::modules::warnings_emit(
                                "Bitwise inversion '~' on bool is deprecated. Use 'not' instead",
                                "DeprecationWarning",
                            );
                            py_int(if *b { -2i64 } else { -1i64 })
                        }
                        _ => return Err(PyError::type_error("bad operand type for unary ~")),
                    }
                };
                self.frames[fi].push(result);
            }
            Opcode::JUMP_FORWARD | Opcode::JUMP | Opcode::JUMP_BACKWARD => {
                let offset = arg as usize;
                match op {
                    Opcode::JUMP_FORWARD => {
                        self.frames[fi].ip += offset;
                    }
                    Opcode::JUMP => {
                        self.frames[fi].ip = offset;
                    }
                    Opcode::JUMP_BACKWARD => {
                        let cur_ip = self.frames[fi].ip;
                        self.frames[fi].ip = cur_ip.wrapping_sub(offset).wrapping_sub(1);
                    }
                    _ => unreachable!(),
                }
            }
            Opcode::POP_JUMP_IF_FALSE => {
                let val = self.frames[fi].pop()?;
                if !val.try_truthy()? {
                    self.frames[fi].ip = arg as usize;
                }
            }
            Opcode::POP_JUMP_IF_TRUE => {
                let val = self.frames[fi].pop()?;
                if val.try_truthy()? {
                    self.frames[fi].ip = arg as usize;
                }
            }
            Opcode::POP_JUMP_IF_NONE => {
                let val = self.frames[fi].pop()?;
                let is_none = matches!(&*val.borrow(), PyObject::None);
                if is_none {
                    self.frames[fi].ip = arg as usize;
                }
            }
            Opcode::POP_JUMP_IF_NOT_NONE => {
                let val = self.frames[fi].pop()?;
                let is_none = matches!(&*val.borrow(), PyObject::None);
                if !is_none {
                    self.frames[fi].ip = arg as usize;
                }
            }
            _ => return Ok(false),
        }
        Ok(true)
    }
}
