use crate::ast::*;
use crate::bytecode::*;
use super::Compiler;
use super::scope::ScopeType;

impl Compiler {
    pub(crate) fn compile_expr(&mut self, expr: &Expr) -> Result<(), String> {
        match expr {
            Expr::Constant(c) => {
                let const_value = match c {
                    Constant::None => ConstValue::None,
                    Constant::Bool(b) => ConstValue::Bool(*b),
                    Constant::Int(s) => ConstValue::Int(s.clone()),
                    Constant::Float(s) => ConstValue::Float(s.clone()),
                    Constant::String(s) => ConstValue::String(s.clone()),
                    Constant::Ellipsis => ConstValue::String("...".to_string()),
                    Constant::Bytes(b) => ConstValue::Bytes(b.clone()),
                    Constant::Complex { real, imag } => ConstValue::Complex {
                        real: real.clone(),
                        imag: imag.clone(),
                    },
                };
                let idx = self.get_const_index(const_value) as u32;
                self.emit(Opcode::LOAD_CONST, idx);
            }
            Expr::Name(name) => {
                // Private-name mangling (`mangle_name`) applies to EVERY
                // identifier that textually occurs within a class
                // definition — not just `self.__attr`-style attribute
                // access (already handled at its own `Expr::Attribute`
                // site) but plain bare-name references too, including
                // ones inside a nested method body (mangling is lexical,
                // not scope-limited — see `mangle_name`'s own doc
                // comment). Missing this was a real regression introduced
                // by the earlier fix that started mangling a `def
                // __method(self):`'s STORAGE key: `Lib/unittest/mock.py`'s
                // `NonCallableMock` defines `def __get_return_value(self):`
                // then references it as a plain name via `property(
                // __get_return_value, __set_return_value)` at class-body
                // level — once the definition's storage became mangled,
                // the unmangled bare-name lookup could no longer find it
                // (`NameError: name '__get_return_value' is not defined`,
                // breaking `unittest.mock` entirely). `mangle_name` itself
                // is a no-op outside a class body / for non-private names,
                // so this is safe everywhere else.
                let name = &self.mangle_name(name);
                if std::env::var("RPY_DEBUG_COMPILE_NAME_RESOLVE")
                    .ok()
                    .as_deref()
                    == Some(name.as_str())
                {
                    eprintln!("NAME_RESOLVE: name={} scope={:?} in_global_names={} freevars={:?} cellvars={:?}", name, self.scope, self.global_names.contains(name), self.code.freevars, self.code.cellvars);
                }
                if self.scope == ScopeType::Module
                    || self.scope == ScopeType::ClassBody
                    || self.global_names.contains(name)
                {
                    let name_idx = self.get_name_index(name) as u32;

                    self.emit(Opcode::LOAD_NAME, name_idx);
                } else if self.scope == ScopeType::Function && self.code.freevars.contains(name) {
                    let fv_idx = self.code.freevars.iter().position(|n| n == name).unwrap();
                    let idx = self.code.cellvars.len() + fv_idx;

                    self.emit(Opcode::LOAD_DEREF, idx as u32);
                } else if self.scope == ScopeType::Function && self.code.cellvars.contains(name) {
                    let idx = self.code.cellvars.iter().position(|n| n == name).unwrap() as u32;

                    self.emit(Opcode::LOAD_DEREF, idx);
                } else if self.scope == ScopeType::Function && self.get_var_index(name).is_some() {
                    let idx = self.get_var_index(name).unwrap() as u32;

                    self.emit(Opcode::LOAD_FAST, idx);
                } else if self.scope == ScopeType::Function {
                    let name_idx = self.get_name_index(name) as u32;

                    self.emit(Opcode::LOAD_GLOBAL, name_idx);
                } else {
                    let name_idx = self.get_name_index(name) as u32;

                    self.emit(Opcode::LOAD_NAME, name_idx);
                }
            }
            Expr::BinOp { left, op, right } => {
                // Constant folding: compute 3+4 etc. at compile time
                if let (Expr::Constant(Constant::Int(a)), Expr::Constant(Constant::Int(b))) =
                    (&**left, &**right)
                {
                    let result = match op {
                        Operator::Add => a
                            .parse::<i64>()
                            .ok()
                            .zip(b.parse::<i64>().ok())
                            .map(|(x, y)| x + y),
                        Operator::Sub => a
                            .parse::<i64>()
                            .ok()
                            .zip(b.parse::<i64>().ok())
                            .map(|(x, y)| x - y),
                        Operator::Mult => a
                            .parse::<i64>()
                            .ok()
                            .zip(b.parse::<i64>().ok())
                            .map(|(x, y)| x * y),
                        _ => None,
                    };
                    if let Some(val) = result {
                        let idx = self.get_const_index(ConstValue::Int(val.to_string())) as u32;
                        self.emit(Opcode::LOAD_CONST, idx);
                        return Ok(());
                    }
                }
                self.compile_expr(left)?;
                self.compile_expr(right)?;
                let bin_op = match op {
                    Operator::Add => 0,
                    Operator::Sub => 1,
                    Operator::Mult => 2,
                    Operator::Div => 3,
                    Operator::FloorDiv => 4,
                    Operator::Mod => 5,
                    Operator::Pow => 6,
                    Operator::LShift => 7,
                    Operator::RShift => 8,
                    Operator::BitOr => 9,
                    Operator::BitXor => 10,
                    Operator::BitAnd => 11,
                    Operator::MatMult => 12,
                };
                self.emit(Opcode::BINARY_OP, bin_op);
            }
            Expr::UnaryOp { op, operand } => {
                // CPython 3.11+ folds `not (a is b)` into a single inverted
                // IS_OP (no UNARY_NOT emitted) — test_positional_only_arg's
                // test_annotations_constant_fold asserts exactly this in the
                // __annotate__ code. Fold the same for `not (a is not b)`.
                if let UnaryOp::Not = op {
                    if let Expr::Compare {
                        left,
                        ops,
                        comparators,
                    } = operand.as_ref()
                    {
                        if ops.len() == 1 && comparators.len() == 1 {
                            match ops[0] {
                                CmpOp::Is | CmpOp::IsNot => {
                                    self.compile_expr(left)?;
                                    self.compile_expr(&comparators[0])?;
                                    let invert = if matches!(ops[0], CmpOp::Is) { 1 } else { 0 };
                                    self.emit(Opcode::IS_OP, invert);
                                    return Ok(());
                                }
                                _ => {}
                            }
                        }
                    }
                }
                self.compile_expr(operand)?;
                match op {
                    UnaryOp::Not => self.emit(Opcode::UNARY_NOT, 0),
                    UnaryOp::UAdd => self.emit(Opcode::UNARY_POSITIVE, 0),
                    UnaryOp::USub => self.emit(Opcode::UNARY_NEGATIVE, 0),
                    UnaryOp::Invert => self.emit(Opcode::UNARY_INVERT, 0),
                }
            }
            Expr::BoolOp { op, values } => {
                let end_label = self.new_label();
                for (i, val) in values.iter().enumerate() {
                    self.compile_expr(val)?;
                    if i < values.len() - 1 {
                        self.emit(Opcode::DUP_TOP, 0);
                        let jump_op = match op {
                            BoolOp::And => Opcode::POP_JUMP_IF_FALSE,
                            BoolOp::Or => Opcode::POP_JUMP_IF_TRUE,
                        };
                        self.emit_jump(jump_op, end_label);
                        self.emit(Opcode::POP_TOP, 0);
                    }
                }
                self.fix_label(end_label);
            }
            Expr::Compare {
                left,
                ops,
                comparators,
            } => {
                let chained_end = self.new_label();
                for (i, (op, right)) in ops.iter().zip(comparators.iter()).enumerate() {
                    if i > 0 {
                        // Chained comparison: re-compile previous comparand as new left
                        self.compile_expr(&comparators[i - 1])?;
                    } else {
                        self.compile_expr(left)?;
                    }
                    self.compile_expr(right)?;
                    let cmp_op = match op {
                        CmpOp::Eq => 2,
                        CmpOp::NotEq => 5,
                        CmpOp::Lt => 0,
                        CmpOp::LtE => 1,
                        CmpOp::Gt => 4,
                        CmpOp::GtE => 3,
                        CmpOp::Is => {
                            // Emit SyntaxWarning for "is" with literal (PEP 8)
                            let actual_left: &Expr = if i == 0 { left } else { &comparators[i - 1] };
                            let is_none_or_bool = |e: &Expr| matches!(e, Expr::Constant(Constant::None) | Expr::Constant(Constant::Bool(_)) | Expr::Constant(Constant::Ellipsis));
                            let left_is_lit = matches!(actual_left, Expr::Constant(_)) && !is_none_or_bool(actual_left);
                            let right_is_lit = matches!(right, Expr::Constant(_)) && !is_none_or_bool(right);
                            if left_is_lit || right_is_lit {
                                let lit_expr = if left_is_lit { actual_left } else { right };
                                let typ_str = match lit_expr {
                                    Expr::Constant(Constant::String(_)) => "str",
                                    Expr::Constant(Constant::Bytes(_)) => "bytes",
                                    Expr::Constant(Constant::Int(_)) => "int",
                                    Expr::Constant(Constant::Float(_)) => "float",
                                    Expr::Constant(Constant::Complex { .. }) => "complex",
                                    Expr::Constant(Constant::Bool(_)) => "bool",
                                    Expr::Constant(Constant::None) => "None",
                                    Expr::Constant(Constant::Ellipsis) => "ellipsis",
                                    _ => "literal",
                                };
                                let msg = format!("\"is\" with '{}' literal. Did you mean \"==\"?", typ_str);
                                if crate::modules::warning_is_error_mode() {
                                    return Err(msg);
                                } else {
                                    crate::modules::warnings_emit(&msg, "SyntaxWarning");
                                }
                            }
                            // CPython 3.11+ has a dedicated IS_OP for
                            // identity (arg 0 = is, 1 = is not); COMPARE_OP
                            // 8/9 were the pre-3.11 encodings.
                            self.emit(Opcode::IS_OP, 0);
                            if i < ops.len() - 1 {
                                self.emit(Opcode::DUP_TOP, 0);
                                self.emit_jump(Opcode::POP_JUMP_IF_FALSE, chained_end);
                                self.emit(Opcode::POP_TOP, 0);
                            }
                            continue;
                        }
                        CmpOp::IsNot => {
                            let actual_left: &Expr = if i == 0 { left } else { &comparators[i - 1] };
                            let is_none_or_bool = |e: &Expr| matches!(e, Expr::Constant(Constant::None) | Expr::Constant(Constant::Bool(_)) | Expr::Constant(Constant::Ellipsis));
                            let left_is_lit = matches!(actual_left, Expr::Constant(_)) && !is_none_or_bool(actual_left);
                            let right_is_lit = matches!(right, Expr::Constant(_)) && !is_none_or_bool(right);
                            if left_is_lit || right_is_lit {
                                let lit_expr = if left_is_lit { actual_left } else { right };
                                let typ_str = match lit_expr {
                                    Expr::Constant(Constant::String(_)) => "str",
                                    Expr::Constant(Constant::Bytes(_)) => "bytes",
                                    Expr::Constant(Constant::Int(_)) => "int",
                                    Expr::Constant(Constant::Float(_)) => "float",
                                    Expr::Constant(Constant::Complex { .. }) => "complex",
                                    Expr::Constant(Constant::Bool(_)) => "bool",
                                    Expr::Constant(Constant::None) => "None",
                                    Expr::Constant(Constant::Ellipsis) => "ellipsis",
                                    _ => "literal",
                                };
                                let msg = format!("\"is\" with '{}' literal. Did you mean \"==\"?", typ_str);
                                if crate::modules::warning_is_error_mode() {
                                    return Err(msg);
                                } else {
                                    crate::modules::warnings_emit(&msg, "SyntaxWarning");
                                }
                            }
                            self.emit(Opcode::IS_OP, 1);
                            if i < ops.len() - 1 {
                                self.emit(Opcode::DUP_TOP, 0);
                                self.emit_jump(Opcode::POP_JUMP_IF_FALSE, chained_end);
                                self.emit(Opcode::POP_TOP, 0);
                            }
                            continue;
                        }
                        CmpOp::In => 6,
                        CmpOp::NotIn => 7,
                    };
                    self.emit(Opcode::COMPARE_OP, cmp_op);
                    if i < ops.len() - 1 {
                        self.emit(Opcode::DUP_TOP, 0);
                        self.emit_jump(Opcode::POP_JUMP_IF_FALSE, chained_end);
                        self.emit(Opcode::POP_TOP, 0);
                    }
                }
                self.fix_label(chained_end);
            }
            Expr::Call {
                func,
                args,
                keywords,
            } => {
                // PEP 3135: super() without args in methods -> super(__class__, self)
                let mut extra_pos = 0usize;
                let is_bare_super = if let Expr::Name(n) = func.as_ref() {
                    n == "super" && args.is_empty() && keywords.is_empty()
                } else {
                    false
                };
                if is_bare_super {
                    // PEP 3135: super() without args resolves against
                    // __class__ (the class this method is textually defined
                    // in) and self — NOT type(self) (the instance's runtime
                    // type), which this used to inject instead. That broke
                    // any 3+-level hierarchy where a method is called on a
                    // subclass instance that doesn't override it: e.g.
                    // Widget -> Input -> HiddenInput, HiddenInput() calling
                    // inherited Input.__init__'s `super().__init__()`. With
                    // cls=type(self)=HiddenInput, "the class after
                    // HiddenInput in HiddenInput's own MRO" is Input —
                    // Input.__init__ again — infinite self-recursion (stack
                    // overflow) instead of reaching Widget.__init__. Since
                    // the class object doesn't exist yet at compile time
                    // (methods compile before the class is built), look it
                    // up by name instead — correct as long as the class is
                    // still bound to its defining name by the time any
                    // instance method actually runs, true for essentially
                    // all real code (`class Foo: ...` always binds "Foo").
                    // Checking `varnames` alone used to wrongly treat a
                    // ZERO-parameter method inside a class body (`class C:
                    // def f(): super()`) as having a `self` to inject via
                    // `LOAD_FAST 0`: `varnames` is non-empty there too, but
                    // only because `__class__` (always added as a cellvar
                    // for any method in a class body, unconditionally,
                    // regardless of whether it even uses `super()`) landed
                    // in slot 0 in the ABSENCE of any real parameter —
                    // `LOAD_FAST 0` then loaded __CLASS__'s cell instead of
                    // a real `self`, raising a confusing `UnboundLocalError`
                    // instead of real CPython's `RuntimeError: super(): no
                    // arguments` for this exact case. `arg_count` (plus
                    // vararg, which also supplies a usable first positional)
                    // is the real signal for "does this function have an
                    // actual first parameter to bind".
                    let has_first_param =
                        self.code.arg_count > 0 || self.code.vararg_name.is_some();
                    if self.scope == ScopeType::Function && has_first_param {
                        if let Some(class_name) = self.class_name_stack.last().cloned() {
                            self.compile_expr(func)?;
                            // PEP 3135: resolve the class through the
                            // `__class__` free var (a cell the class body
                            // owns, filled by __build_class__) — NOT
                            // LOAD_GLOBAL of the class name, which fails
                            // for a class defined inside a function (its
                            // name is a local there, not a global).
                            let class_idx =
                                self.code.freevars.iter().position(|n| n == "__class__");
                            if let Some(fv_idx) = class_idx {
                                let deref_idx = self.code.cellvars.len() + fv_idx;
                                if std::env::var("RPY_DEBUG_SUPER").is_ok() {
                                    eprintln!("SUPER LOAD_DEREF class={} fv_idx={} cellvars={:?} freevars={:?}", class_name, fv_idx, self.code.cellvars, self.code.freevars);
                                }
                                self.emit(Opcode::LOAD_DEREF, deref_idx as u32);
                            } else {
                                if std::env::var("RPY_DEBUG_SUPER").is_ok() {
                                    eprintln!("SUPER LOAD_GLOBAL fallback class={} cellvars={:?} freevars={:?}", class_name, self.code.cellvars, self.code.freevars);
                                }
                                // Fallback: class name as a global (module-
                                // level classes still work even if the
                                // __class__ cell didn't get wired up).
                                let class_name_idx = self.get_name_index(&class_name) as u32;
                                self.emit(Opcode::LOAD_GLOBAL, class_name_idx);
                            }
                            self.emit(Opcode::LOAD_FAST, 0);
                            extra_pos = 2;
                        } else {
                            // Bare super() outside a class body is invalid
                            // Python anyway; keep the old best-effort behavior.
                            self.compile_expr(func)?;
                            let type_name_idx = self.get_name_index("type") as u32;
                            self.emit(Opcode::LOAD_GLOBAL, type_name_idx);
                            self.emit(Opcode::LOAD_FAST, 0);
                            self.emit(Opcode::CALL, 1);
                            self.emit(Opcode::LOAD_FAST, 0);
                            extra_pos = 2;
                        }
                    } else {
                        // The enclosing function takes NO parameters at all
                        // (`self.code.varnames.is_empty()`) — there is
                        // genuinely no `self` to bind a bare `super()` to,
                        // matching real CPython's `RuntimeError: super():
                        // no arguments` (found via CPython's own
                        // `test_super.py::test_obscure_super_errors`).
                        // Emitting the ordinary `super` global here would
                        // instead hit the generic, wrong `TypeError:
                        // super() requires 2 arguments` once called with
                        // zero args — call a dedicated internal helper that
                        // raises the correct error/type directly instead.
                        let name_idx = self.get_name_index("__super_no_arguments_error") as u32;
                        self.emit(Opcode::LOAD_GLOBAL, name_idx);
                    }
                } else {
                    self.compile_expr(func)?;
                }
                // f(*args) / f(x, *args) / f(**kwargs): a starred positional
                // or bare-** keyword must be unpacked at call time, not
                // passed through as a single tuple/dict value — the plain
                // CALL opcode below has no way to express "this many of my
                // arguments came from unpacking", so use CALL_FUNCTION_EX
                // instead, matching CPython's own split between the two.
                let has_star_args = args.iter().any(|a| matches!(a, Expr::Starred(_)));
                let has_star_kwargs = keywords.iter().any(|kw| kw.arg.is_none());
                if has_star_args || has_star_kwargs {
                    self.emit(Opcode::BUILD_LIST, 0);
                    for arg in args {
                        if let Expr::Starred(inner) = arg {
                            self.compile_expr(inner)?;
                            self.emit(Opcode::LIST_EXTEND, 0);
                        } else {
                            self.compile_expr(arg)?;
                            self.emit(Opcode::LIST_APPEND, 0);
                        }
                    }
                    self.emit(Opcode::LIST_TO_TUPLE, 0);
                    self.emit(Opcode::BUILD_MAP, 0);
                    for kw in keywords {
                        if let Some(name) = &kw.arg {
                            let name_idx =
                                self.get_const_index(ConstValue::String(name.clone())) as u32;
                            self.emit(Opcode::LOAD_CONST, name_idx);
                            self.compile_expr(&kw.value)?;
                            self.emit(Opcode::MAP_ADD, 0);
                        } else {
                            self.compile_expr(&kw.value)?;
                            self.emit(Opcode::DICT_MERGE, 0);
                        }
                    }
                    self.emit(Opcode::CALL_FUNCTION_EX, 0);
                    return Ok(());
                }

                let npos = args.len() + extra_pos;
                let nkw = keywords.len();

                for arg in args {
                    self.compile_expr(arg)?;
                }
                for kw in keywords {
                    if let Some(name) = &kw.arg {
                        let name_idx =
                            self.get_const_index(ConstValue::String(name.clone())) as u32;
                        self.emit(Opcode::LOAD_CONST, name_idx);
                        self.compile_expr(&kw.value)?;
                    } else {
                        // **kwargs
                        self.compile_expr(&kw.value)?;
                    }
                }
                let call_arg = npos | (nkw << 8);
                self.emit(Opcode::CALL, call_arg as u32);
            }
            Expr::IfExp { test, body, orelse } => {
                self.compile_expr(test)?;
                let else_label = self.new_label();
                let end_label = self.new_label();
                self.emit_jump(Opcode::POP_JUMP_IF_FALSE, else_label);
                self.compile_expr(body)?;
                self.emit_jump(Opcode::JUMP, end_label);
                self.fix_label(else_label);
                self.compile_expr(orelse)?;
                self.fix_label(end_label);
            }
            Expr::Lambda { args, body } => {
                self.compile_function(
                    "<lambda>".to_string(),
                    args,
                    &[Stmt::Return(Some(body.clone()))],
                    false,
                    &None,
                )?;
            }
            Expr::Attribute { value, attr } => {
                self.compile_expr(value)?;
                let idx = self.get_name_index(&self.mangle_name(attr)) as u32;
                self.emit(Opcode::LOAD_ATTR, idx);
            }
            Expr::Subscript { value, slice } => {
                self.compile_expr(value)?;
                self.compile_expr(slice)?;
                self.emit(Opcode::BINARY_OP, 13); // SUBSCR = 13
            }
            Expr::List(elts) => {
                if elts.iter().any(|e| matches!(e, Expr::Starred(_))) {
                    // Star unpacking present — use incremental building with LIST_APPEND/LIST_EXTEND
                    self.emit(Opcode::BUILD_LIST, 0);
                    for elt in elts {
                        if let Expr::Starred(inner) = elt {
                            self.compile_expr(inner)?;
                            self.emit(Opcode::LIST_EXTEND, 0);
                        } else {
                            self.compile_expr(elt)?;
                            self.emit(Opcode::LIST_APPEND, 0);
                        }
                    }
                } else {
                    for elt in elts {
                        self.compile_expr(elt)?;
                    }
                    self.emit(Opcode::BUILD_LIST, elts.len() as u32);
                }
            }
            Expr::Tuple(elts) => {
                if elts.iter().any(|e| matches!(e, Expr::Starred(_))) {
                    // Star unpacking present — build incrementally as a list
                    // (LIST_APPEND/LIST_EXTEND handle Starred correctly),
                    // then convert to a tuple.
                    self.emit(Opcode::BUILD_LIST, 0);
                    for elt in elts {
                        if let Expr::Starred(inner) = elt {
                            self.compile_expr(inner)?;
                            self.emit(Opcode::LIST_EXTEND, 0);
                        } else {
                            self.compile_expr(elt)?;
                            self.emit(Opcode::LIST_APPEND, 0);
                        }
                    }
                    self.emit(Opcode::LIST_TO_TUPLE, 0);
                } else {
                    for elt in elts {
                        self.compile_expr(elt)?;
                    }
                    self.emit(Opcode::BUILD_TUPLE, elts.len() as u32);
                }
            }
            Expr::Dict { keys, values } => {
                self.emit(Opcode::BUILD_MAP, 0);
                let mut dup_count = 0u32;
                for (key, value) in keys.iter().zip(values.iter()) {
                    match key {
                        Some(k) => {
                            self.emit(Opcode::DUP_TOP, 0);
                            dup_count += 1;
                            self.compile_expr(k)?;
                            self.compile_expr(value)?;
                            self.emit(Opcode::MAP_ADD, 1);
                        }
                        None => {
                            // Dict unpacking: {**expr}
                            self.emit(Opcode::DUP_TOP, 0);
                            dup_count += 1;
                            self.compile_expr(value)?;
                            self.emit(Opcode::DICT_MERGE, 1);
                        }
                    }
                }
                // Pop all DUP_TOP copies except the original BUILD_MAP result
                for _ in 0..dup_count {
                    self.emit(Opcode::POP_TOP, 0);
                }
            }
            Expr::Set(elts) => {
                if elts.iter().any(|e| matches!(e, Expr::Starred(_))) {
                    // Star unpacking present (`{*a, *b}`) — build incrementally,
                    // mirroring the Tuple case above.
                    self.emit(Opcode::BUILD_SET, 0);
                    for elt in elts {
                        if let Expr::Starred(inner) = elt {
                            self.compile_expr(inner)?;
                            self.emit(Opcode::SET_UPDATE, 0);
                        } else {
                            self.compile_expr(elt)?;
                            self.emit(Opcode::SET_ADD, 0);
                        }
                    }
                } else {
                    for elt in elts {
                        self.compile_expr(elt)?;
                    }
                    self.emit(Opcode::BUILD_SET, elts.len() as u32);
                }
            }
            Expr::ListComp { elt, generators } => {
                self.compile_comprehension(elt, generators, false)?;
            }
            Expr::SetComp { elt, generators } => {
                self.compile_comprehension(elt, generators, true)?;
            }
            Expr::GeneratorExp { elt, generators } => {
                self.compile_comprehension(elt, generators, false)?;
                self.emit(Opcode::GET_ITER, 0);
            }
            Expr::Slice { lower, upper, step } => {
                let const_none = self.get_const_index(ConstValue::None) as u32;
                if let Some(l) = lower {
                    self.compile_expr(l)?;
                } else {
                    self.emit(Opcode::LOAD_CONST, const_none);
                }
                if let Some(u) = upper {
                    self.compile_expr(u)?;
                } else {
                    self.emit(Opcode::LOAD_CONST, const_none);
                }
                if step.is_some() {
                    if let Some(s) = step {
                        self.compile_expr(s)?;
                    }
                    self.emit(Opcode::BUILD_SLICE, 3);
                } else {
                    self.emit(Opcode::BUILD_SLICE, 2);
                }
            }
            Expr::Starred(expr) => {
                self.compile_expr(expr)?;
            }
            Expr::Yield(Some(expr)) => {
                if self.scope != ScopeType::Function {
                    return Err("'yield' outside function".to_string());
                }
                self.compile_expr(expr)?;
                self.emit(Opcode::YIELD_VALUE, 0);
            }
            Expr::Yield(None) => {
                if self.scope != ScopeType::Function {
                    return Err("'yield' outside function".to_string());
                }
                let const_none = self.get_const_index(ConstValue::None) as u32;
                self.emit(Opcode::LOAD_CONST, const_none);
                self.emit(Opcode::YIELD_VALUE, 0);
            }
            Expr::FString(parts) => {
                let mut count = 0usize;
                for part in parts {
                    match part {
                        FStringPart::String(s) => {
                            self.compile_expr(&Expr::Constant(Constant::String(s.clone())))?;
                            count += 1;
                        }
                        FStringPart::Expr {
                            expr,
                            conversion,
                            format_spec,
                        } => {
                            self.compile_expr(&expr)?;
                            if *conversion != 0 {
                                self.emit(Opcode::CONVERT_VALUE, *conversion as u32);
                            }
                            if let Some(spec) = format_spec {
                                self.compile_expr(&spec)?;
                                self.emit(Opcode::FORMAT_WITH_SPEC, 0);
                            } else if *conversion == 0 {
                                self.emit(Opcode::FORMAT_SIMPLE, 0);
                            }
                            count += 1;
                        }
                    }
                }
                if count > 1 {
                    self.emit(Opcode::BUILD_STRING, count as u32);
                }
            }
            Expr::JoinedStr(parts) => {
                for part in parts {
                    self.compile_expr(part)?;
                }
                if parts.len() == 1 {
                    // Already on stack
                } else {
                    self.emit(Opcode::BUILD_STRING, parts.len() as u32);
                }
            }
            Expr::NamedExpr { target, value } => {
                self.compile_expr(value)?;
                self.emit(Opcode::DUP_TOP, 0);
                self.compile_assign_target(target)?;
            }
            Expr::DictComp {
                key,
                value,
                generators,
            } => {
                self.compile_dict_comprehension(key, value, generators)?;
            }
            Expr::YieldFrom(expr) => {
                // Mark this code object so the VM knows FOR_ITER loops here
                // belong to a `yield from` (generator.throw() delegation).
                self.code.flags |= 0x0200;
                // Simple yield from: iterate and yield each value
                self.compile_expr(expr)?;
                self.emit(Opcode::GET_ITER, 0);
                let end_label = self.new_label();
                let loop_label = self.new_label();
                self.mark_label(loop_label);
                self.emit_jump(Opcode::FOR_ITER, end_label);
                self.emit(Opcode::YIELD_VALUE, 0);
                self.emit(Opcode::POP_TOP, 0);
                self.emit_backward_jump(loop_label);
                self.fix_label(end_label);
                self.emit(Opcode::POP_ITER, 0);
                let const_none = self.get_const_index(ConstValue::None) as u32;
                self.emit(Opcode::LOAD_CONST, const_none);
            }
            Expr::Await(expr) => {
                self.compile_expr(expr)?;
                self.emit(Opcode::GET_AWAITABLE, 0);
                let const_none = self.get_const_index(ConstValue::None) as u32;
                self.emit(Opcode::LOAD_CONST, const_none);
                // Emit the SEND/YIELD loop (matching CPython's `await` compilation):
                //   >> loop:
                //     SEND cleanup_target   (jump to cleanup_target on StopIteration)
                //     YIELD_VALUE           (yield the awaited value)
                //     JUMP_BACKWARD loop    (the resume value from send() is used
                //                           as the next SEND value)
                //   >> cleanup_target:
                //     END_SEND             (pop iterator, pop result, push result)
                let loop_label = self.new_label();
                let end_label = self.new_label();
                self.mark_label(loop_label);
                self.emit_jump(Opcode::SEND, end_label);
                self.emit(Opcode::YIELD_VALUE, 0);
                self.emit_backward_jump(loop_label);
                self.fix_label(end_label);
                self.emit(Opcode::END_SEND, 0);
            }
        }
        Ok(())
    }

}
