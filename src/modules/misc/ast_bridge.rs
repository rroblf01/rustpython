//! Bridge: this interpreter's own parser (`crate::parser::Parser`, whose
//! AST lives in `crate::ast`) → CPython-compatible `ast.*` node trees
//! (the Python classes in `Lib/ast.py`).
//!
//! Backs `ast.parse()` (via `_ast_native._parse_tree`). Nodes are built as
//! direct `PyObject::Instance`s of the `ast.*` classes (no per-node VM
//! re-entry — hundreds of nodes per parse would make that path
//! prohibitively slow) with all `_fields` filled plus
//! `lineno`/`col_offset`/`end_lineno`/`end_col_offset` (statements carry
//! real lines via `Stmt::Located`; expressions default to 1:0 since the
//! internal AST tracks no expression positions).

use crate::ast::*;
use crate::object::*;
pub struct AstBuilder {
    module: PyObjectRef,
    /// `ast.PyCF_OPTIMIZED_AST`: CPython's optimizer rewrites `__debug__`
    /// into a `Constant` (and strips docstrings) before handing back the
    /// tree — `compile(src, f, 'exec', flags=ast.PyCF_OPTIMIZED_AST)`
    /// yields `Constant(value=__debug__)` where the plain `ONLY_AST` tree
    /// still has a `Name` (test_builtin's `test_compile_ast`).
    optimize_ast: bool,
}

impl AstBuilder {
    pub fn new() -> PyResult<Self> {
        Self::with_optimize(false)
    }

    pub fn with_optimize(optimize_ast: bool) -> PyResult<Self> {
        let m = crate::modules::get_module("ast")
            .ok_or_else(|| PyError::ImportError("ast module not loaded".to_string()))?;
        Ok(Self {
            module: m,
            optimize_ast,
        })
    }

    fn cls(&self, name: &str) -> PyResult<PyObjectRef> {
        self.module.borrow().get_attribute(name)
    }

    /// Build an `ast.*` node: direct Instance of the named class with the
    /// given fields plus default 1:0-1:0 locations (overridden via `loc`
    /// for statements carrying real lines).
    fn mk(&self, name: &str, fields: Vec<(&str, PyObjectRef)>) -> PyResult<PyObjectRef> {
        let cls = self.cls(name)?;
        let typ = match &*cls.borrow() {
            PyObject::Type { .. } => cls.clone(),
            _ => {
                return Err(PyError::type_error(format!("ast.{} is not a class", name)));
            }
        };
        let mut dict = AttrMap::new();
        for (k, v) in fields {
            dict.insert_str(k, v);
        }
        dict.insert_str("lineno", py_int(1));
        dict.insert_str("col_offset", py_int(0));
        dict.insert_str("end_lineno", py_int(1));
        dict.insert_str("end_col_offset", py_int(0));
        Ok(PyObjectRef::new(PyObject::Instance { typ, dict }))
    }

    fn set_line(&self, node: &PyObjectRef, line: usize) {
        if let PyObject::Instance { dict, .. } = &mut *node.borrow_mut() {
            dict.insert_str("lineno", py_int(line as i64));
            dict.insert_str("end_lineno", py_int(line as i64));
        }
    }

    /// Stamp a built node with a source span from `Expr::position`.
    /// Lexer columns are 1-indexed; `col_offset`/`end_col_offset` are
    /// 0-indexed like CPython's.
    fn set_span(
        &self,
        node: &PyObjectRef,
        line: usize,
        col: usize,
        end_line: usize,
        end_col: usize,
    ) {
        if let PyObject::Instance { dict, .. } = &mut *node.borrow_mut() {
            dict.insert_str("lineno", py_int(line as i64));
            dict.insert_str("col_offset", py_int(col.saturating_sub(1) as i64));
            dict.insert_str("end_lineno", py_int(end_line as i64));
            dict.insert_str("end_col_offset", py_int(end_col.saturating_sub(1) as i64));
        }
    }

    fn const_none(&self) -> PyObjectRef {
        py_none()
    }

    pub fn build_program(&self, prog: &Program, mode: &str) -> PyResult<PyObjectRef> {
        match (prog, mode) {
            (Program::Module(stmts), "exec" | "single") => {
                let body: Vec<PyObjectRef> = stmts
                    .iter()
                    .map(|s| self.build_stmt_outer(s))
                    .collect::<PyResult<_>>()?;
                let root = if mode == "single" {
                    self.mk("Interactive", vec![("body", py_list(body))])?
                } else {
                    self.mk(
                        "Module",
                        vec![("body", py_list(body)), ("type_ignores", py_list(vec![]))],
                    )?
                };
                Ok(root)
            }
            (Program::Module(stmts), "eval") => {
                // `eval` mode with statements is a syntax error upstream;
                // our parser accepts it — take the single expression.
                if stmts.len() == 1 {
                    if let Stmt::Expr(e) = Self::unwrap(stmts.first().unwrap()) {
                        let body = self.build_expr(e, "Load")?;
                        return self.mk("Expression", vec![("body", body)]);
                    }
                }
                Err(PyError::syntax_error("invalid syntax".to_string()))
            }
            (Program::Expression(e), _) => {
                let body = self.build_expr(e, "Load")?;
                self.mk("Expression", vec![("body", body)])
            }
            (Program::Module(_), _) => Err(PyError::syntax_error(format!(
                "invalid mode {:?} (expected exec/eval/single)",
                mode
            ))),
        }
    }

    fn unwrap(s: &Stmt) -> &Stmt {
        if let Stmt::Located(_, inner) = s {
            inner
        } else {
            s
        }
    }

    fn build_stmt_outer(&self, s: &Stmt) -> PyResult<PyObjectRef> {
        match s {
            Stmt::Located(line, inner) => {
                let node = self.build_stmt(inner, *line)?;
                self.set_line(&node, *line);
                Ok(node)
            }
            _ => self.build_stmt(s, 1),
        }
    }

    fn build_body(&self, stmts: &[Stmt]) -> PyResult<PyObjectRef> {
        let v: Vec<PyObjectRef> = stmts
            .iter()
            .map(|s| self.build_stmt_outer(s))
            .collect::<PyResult<_>>()?;
        Ok(py_list(v))
    }

    fn build_stmt(&self, s: &Stmt, line: usize) -> PyResult<PyObjectRef> {
        let _ = line;
        match s {
            Stmt::Located(_, inner) => self.build_stmt(inner, 1),
            Stmt::FunctionDef {
                name,
                args,
                body,
                decorator_list,
                returns,
                is_async,
                type_params,
            } => {
                let cls = if *is_async {
                    "AsyncFunctionDef"
                } else {
                    "FunctionDef"
                };
                let node = self.mk(
                    cls,
                    vec![
                        ("name", py_str(name)),
                        ("args", self.build_arguments(args)?),
                        ("body", self.build_body(body)?),
                        (
                            "decorator_list",
                            self.build_expr_list(decorator_list, "Load")?,
                        ),
                        (
                            "returns",
                            match returns {
                                Some(r) => self.build_expr(r, "Load")?,
                                None => self.const_none(),
                            },
                        ),
                        ("type_comment", self.const_none()),
                        (
                            "type_params",
                            py_list(
                                type_params
                                    .iter()
                                    .map(|t| {
                                        self.mk(
                                            "TypeVar",
                                            vec![
                                                ("name", py_str(t)),
                                                ("bound", self.const_none()),
                                                ("default_value", self.const_none()),
                                            ],
                                        )
                                    })
                                    .collect::<PyResult<_>>()?,
                            ),
                        ),
                    ],
                )?;
                Ok(node)
            }
            Stmt::ClassDef {
                name,
                bases,
                keywords,
                body,
                decorator_list,
                type_params,
            } => self.mk(
                "ClassDef",
                vec![
                    ("name", py_str(name)),
                    ("bases", self.build_expr_list(bases, "Load")?),
                    (
                        "keywords",
                        py_list(
                            keywords
                                .iter()
                                .map(|k| {
                                    self.mk(
                                        "keyword",
                                        vec![
                                            (
                                                "arg",
                                                match &k.arg {
                                                    Some(a) => py_str(a),
                                                    None => self.const_none(),
                                                },
                                            ),
                                            ("value", self.build_expr(&k.value, "Load")?),
                                        ],
                                    )
                                })
                                .collect::<PyResult<Vec<_>>>()?,
                        ),
                    ),
                    ("body", self.build_body(body)?),
                    (
                        "decorator_list",
                        self.build_expr_list(decorator_list, "Load")?,
                    ),
                    (
                        "type_params",
                        py_list(
                            type_params
                                .iter()
                                .map(|t| {
                                    self.mk(
                                        "TypeVar",
                                        vec![
                                            ("name", py_str(t)),
                                            ("bound", self.const_none()),
                                            ("default_value", self.const_none()),
                                        ],
                                    )
                                })
                                .collect::<PyResult<_>>()?,
                        ),
                    ),
                ],
            ),
            Stmt::Return(v) => self.mk(
                "Return",
                vec![(
                    "value",
                    match v {
                        Some(e) => self.build_expr(e, "Load")?,
                        None => self.const_none(),
                    },
                )],
            ),
            Stmt::Delete(targets) => self.mk(
                "Delete",
                vec![("targets", self.build_expr_list(targets, "Del")?)],
            ),
            Stmt::Assign { targets, value } => self.mk(
                "Assign",
                vec![
                    ("targets", self.build_expr_list(targets, "Store")?),
                    ("value", self.build_expr(value, "Load")?),
                    ("type_comment", self.const_none()),
                ],
            ),
            Stmt::AugAssign { target, op, value } => self.mk(
                "AugAssign",
                vec![
                    ("target", self.build_expr(target, "Store")?),
                    ("op", self.build_operator(op)?),
                    ("value", self.build_expr(value, "Load")?),
                ],
            ),
            Stmt::AnnAssign {
                target,
                annotation,
                value,
            } => {
                let simple = matches!(&**target, Expr::Name(_)) as i64;
                self.mk(
                    "AnnAssign",
                    vec![
                        ("target", self.build_expr(target, "Store")?),
                        ("annotation", self.build_expr(annotation, "Load")?),
                        (
                            "value",
                            match value {
                                Some(v) => self.build_expr(v, "Load")?,
                                None => self.const_none(),
                            },
                        ),
                        ("simple", py_int(simple)),
                    ],
                )
            }
            Stmt::For {
                target,
                iter,
                body,
                orelse,
                is_async,
            } => {
                let cls = if *is_async { "AsyncFor" } else { "For" };
                self.mk(
                    cls,
                    vec![
                        ("target", self.build_expr(target, "Store")?),
                        ("iter", self.build_expr(iter, "Load")?),
                        ("body", self.build_body(body)?),
                        ("orelse", self.build_body(orelse)?),
                        ("type_comment", self.const_none()),
                    ],
                )
            }
            Stmt::While { test, body, orelse } => self.mk(
                "While",
                vec![
                    ("test", self.build_expr(test, "Load")?),
                    ("body", self.build_body(body)?),
                    ("orelse", self.build_body(orelse)?),
                ],
            ),
            Stmt::If { test, body, orelse } => self.mk(
                "If",
                vec![
                    ("test", self.build_expr(test, "Load")?),
                    ("body", self.build_body(body)?),
                    ("orelse", self.build_body(orelse)?),
                ],
            ),
            Stmt::With {
                items,
                body,
                is_async,
            } => {
                let cls = if *is_async { "AsyncWith" } else { "With" };
                self.mk(
                    cls,
                    vec![
                        (
                            "items",
                            py_list(
                                items
                                    .iter()
                                    .map(|it| {
                                        self.mk(
                                            "withitem",
                                            vec![
                                                (
                                                    "context_expr",
                                                    self.build_expr(&it.context_expr, "Load")?,
                                                ),
                                                (
                                                    "optional_vars",
                                                    match &it.optional_vars {
                                                        Some(v) => self.build_expr(v, "Store")?,
                                                        None => self.const_none(),
                                                    },
                                                ),
                                            ],
                                        )
                                    })
                                    .collect::<PyResult<_>>()?,
                            ),
                        ),
                        ("body", self.build_body(body)?),
                        ("type_comment", self.const_none()),
                    ],
                )
            }
            Stmt::Match { subject, cases } => self.mk(
                "Match",
                vec![
                    ("subject", self.build_expr(subject, "Load")?),
                    (
                        "cases",
                        py_list(
                            cases
                                .iter()
                                .map(|c| {
                                    self.mk(
                                        "match_case",
                                        vec![
                                            ("pattern", self.build_pattern(&c.pattern)?),
                                            (
                                                "guard",
                                                match &c.guard {
                                                    Some(g) => self.build_expr(g, "Load")?,
                                                    None => self.const_none(),
                                                },
                                            ),
                                            ("body", self.build_body(&c.body)?),
                                        ],
                                    )
                                })
                                .collect::<PyResult<_>>()?,
                        ),
                    ),
                ],
            ),
            Stmt::Raise { exc, cause } => self.mk(
                "Raise",
                vec![
                    (
                        "exc",
                        match exc {
                            Some(e) => self.build_expr(e, "Load")?,
                            None => self.const_none(),
                        },
                    ),
                    (
                        "cause",
                        match cause {
                            Some(c) => self.build_expr(c, "Load")?,
                            None => self.const_none(),
                        },
                    ),
                ],
            ),
            Stmt::Try {
                body,
                handlers,
                handlers_star,
                orelse,
                finalbody,
            } => {
                // CPython forbids mixing `except*` with plain `except`;
                // route to TryStar when star-handlers are present.
                if handlers_star.is_empty() {
                    let hs: Vec<PyObjectRef> = handlers
                        .iter()
                        .map(|h| self.build_except_handler(&h.typ, &h.name, &h.body))
                        .collect::<PyResult<_>>()?;
                    self.mk(
                        "Try",
                        vec![
                            ("body", self.build_body(body)?),
                            ("handlers", py_list(hs)),
                            ("orelse", self.build_body(orelse)?),
                            ("finalbody", self.build_body(finalbody)?),
                        ],
                    )
                } else {
                    let hs: Vec<PyObjectRef> = handlers_star
                        .iter()
                        .map(|h| self.build_except_handler(&h.typ, &h.name, &h.body))
                        .collect::<PyResult<_>>()?;
                    self.mk(
                        "TryStar",
                        vec![
                            ("body", self.build_body(body)?),
                            ("handlers", py_list(hs)),
                            ("orelse", self.build_body(orelse)?),
                            ("finalbody", self.build_body(finalbody)?),
                        ],
                    )
                }
            }
            Stmt::Assert { test, msg } => self.mk(
                "Assert",
                vec![
                    ("test", self.build_expr(test, "Load")?),
                    (
                        "msg",
                        match msg {
                            Some(m) => self.build_expr(m, "Load")?,
                            None => self.const_none(),
                        },
                    ),
                ],
            ),
            Stmt::Import(aliases) => {
                self.mk("Import", vec![("names", self.build_alias_list(aliases)?)])
            }
            Stmt::ImportFrom {
                module,
                names,
                level,
            } => self.mk(
                "ImportFrom",
                vec![
                    (
                        "module",
                        match module {
                            Some(m) => py_str(m),
                            None => self.const_none(),
                        },
                    ),
                    ("names", self.build_alias_list(names)?),
                    ("level", py_int(level.unwrap_or(0) as i64)),
                ],
            ),
            Stmt::Global(names) => self.mk(
                "Global",
                vec![("names", py_list(names.iter().map(|n| py_str(n)).collect()))],
            ),
            Stmt::Nonlocal(names) => self.mk(
                "Nonlocal",
                vec![("names", py_list(names.iter().map(|n| py_str(n)).collect()))],
            ),
            Stmt::Expr(e) => {
                let v = self.build_expr(e, "Load")?;
                self.mk("Expr", vec![("value", v)])
            }
            Stmt::Pass => self.mk("Pass", vec![]),
            Stmt::Break => self.mk("Break", vec![]),
            Stmt::Continue => self.mk("Continue", vec![]),
            Stmt::TypeAlias {
                name,
                type_params,
                value,
            } => self.mk(
                "TypeAlias",
                vec![
                    (
                        "name",
                        self.mk(
                            "Name",
                            vec![("id", py_str(name)), ("ctx", self.mk("Load", vec![])?)],
                        )?,
                    ),
                    (
                        "type_params",
                        py_list(
                            type_params
                                .iter()
                                .map(|t| {
                                    self.mk(
                                        "TypeVar",
                                        vec![
                                            ("name", py_str(t)),
                                            ("bound", self.const_none()),
                                            ("default_value", self.const_none()),
                                        ],
                                    )
                                })
                                .collect::<PyResult<_>>()?,
                        ),
                    ),
                    ("value", self.build_expr(value, "Load")?),
                ],
            ),
        }
    }

    fn build_except_handler(
        &self,
        typ: &Option<Box<Expr>>,
        name: &Option<Ident>,
        body: &[Stmt],
    ) -> PyResult<PyObjectRef> {
        self.mk(
            "ExceptHandler",
            vec![
                (
                    "type",
                    match typ {
                        Some(t) => self.build_expr(t, "Load")?,
                        None => self.const_none(),
                    },
                ),
                (
                    "name",
                    match name {
                        Some(n) => py_str(n),
                        None => self.const_none(),
                    },
                ),
                ("body", self.build_body(body)?),
            ],
        )
    }

    fn build_alias_list(&self, aliases: &[Alias]) -> PyResult<PyObjectRef> {
        Ok(py_list(
            aliases
                .iter()
                .map(|a| {
                    self.mk(
                        "alias",
                        vec![
                            ("name", py_str(&a.name)),
                            (
                                "asname",
                                match &a.asname {
                                    Some(n) => py_str(n),
                                    None => self.const_none(),
                                },
                            ),
                        ],
                    )
                })
                .collect::<PyResult<_>>()?,
        ))
    }

    fn build_expr_list(&self, exprs: &[Expr], ctx: &str) -> PyResult<PyObjectRef> {
        Ok(py_list(
            exprs
                .iter()
                .map(|e| self.build_expr(e, ctx))
                .collect::<PyResult<_>>()?,
        ))
    }

    fn build_arguments(&self, args: &[Arg]) -> PyResult<PyObjectRef> {
        let mut posonly: Vec<PyObjectRef> = vec![];
        let mut plain: Vec<PyObjectRef> = vec![];
        let mut defaults: Vec<PyObjectRef> = vec![];
        let mut vararg: PyObjectRef = self.const_none();
        let mut kwonly: Vec<PyObjectRef> = vec![];
        let mut kw_defaults: Vec<PyObjectRef> = vec![];
        let mut kwarg: PyObjectRef = self.const_none();
        for a in args {
            if a.is_vararg {
                vararg = self.build_arg(a)?;
                continue;
            }
            if a.is_kwarg {
                kwarg = self.build_arg(a)?;
                continue;
            }
            let node = self.build_arg(a)?;
            if a.is_posonlyarg {
                posonly.push(node);
                if let Some(d) = &a.default {
                    defaults.push(self.build_expr(d, "Load")?);
                }
            } else if a.is_kwonly {
                kwonly.push(node);
                match &a.default {
                    Some(d) => kw_defaults.push(self.build_expr(d, "Load")?),
                    None => kw_defaults.push(self.const_none()),
                }
            } else {
                if let Some(d) = &a.default {
                    let _ = d;
                }
                plain.push(node);
            }
        }
        // defaults align to the LAST N positional (posonly+plain) params.
        let mut positional: Vec<(&Arg, PyObjectRef)> = vec![];
        for a in args {
            if a.is_vararg || a.is_kwarg || a.is_kwonly {
                continue;
            }
            positional.push((a, self.build_arg(a)?));
        }
        let n_defaulted = positional
            .iter()
            .filter(|(a, _)| a.default.is_some())
            .count();
        let mut plain2: Vec<PyObjectRef> = vec![];
        let mut posonly2: Vec<PyObjectRef> = vec![];
        for (a, node) in &positional {
            if a.is_posonlyarg {
                posonly2.push(node.clone());
            } else {
                plain2.push(node.clone());
            }
        }
        let mut defaults2: Vec<PyObjectRef> = vec![];
        for (a, _) in positional
            .iter()
            .rev()
            .take(n_defaulted)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
        {
            if let Some(d) = &a.default {
                defaults2.push(self.build_expr(d, "Load")?);
            }
        }
        let _ = (plain, defaults, posonly);
        self.mk(
            "arguments",
            vec![
                ("posonlyargs", py_list(posonly2)),
                ("args", py_list(plain2)),
                ("vararg", vararg),
                ("kwonlyargs", py_list(kwonly)),
                ("kw_defaults", py_list(kw_defaults)),
                ("kwarg", kwarg),
                ("defaults", py_list(defaults2)),
            ],
        )
    }

    fn build_arg(&self, a: &Arg) -> PyResult<PyObjectRef> {
        self.mk(
            "arg",
            vec![
                ("arg", py_str(&a.arg)),
                (
                    "annotation",
                    match &a.annotation {
                        Some(e) => self.build_expr(e, "Load")?,
                        None => self.const_none(),
                    },
                ),
                ("type_comment", self.const_none()),
            ],
        )
    }

    fn build_operator(&self, op: &Operator) -> PyResult<PyObjectRef> {
        let name = match op {
            Operator::Add => "Add",
            Operator::Sub => "Sub",
            Operator::Mult => "Mult",
            Operator::Div => "Div",
            Operator::FloorDiv => "FloorDiv",
            Operator::Mod => "Mod",
            Operator::Pow => "Pow",
            Operator::LShift => "LShift",
            Operator::RShift => "RShift",
            Operator::BitOr => "BitOr",
            Operator::BitXor => "BitXor",
            Operator::BitAnd => "BitAnd",
            Operator::MatMult => "MatMult",
        };
        self.mk(name, vec![])
    }

    fn build_unaryop(&self, op: &UnaryOp) -> PyResult<PyObjectRef> {
        let name = match op {
            UnaryOp::Invert => "Invert",
            UnaryOp::Not => "Not",
            UnaryOp::UAdd => "UAdd",
            UnaryOp::USub => "USub",
        };
        self.mk(name, vec![])
    }

    fn build_cmpop(&self, op: &CmpOp) -> PyResult<PyObjectRef> {
        let name = match op {
            CmpOp::Eq => "Eq",
            CmpOp::NotEq => "NotEq",
            CmpOp::Lt => "Lt",
            CmpOp::LtE => "LtE",
            CmpOp::Gt => "Gt",
            CmpOp::GtE => "GtE",
            CmpOp::Is => "Is",
            CmpOp::IsNot => "IsNot",
            CmpOp::In => "In",
            CmpOp::NotIn => "NotIn",
        };
        self.mk(name, vec![])
    }

    fn build_ctx(&self, ctx: &str) -> PyResult<PyObjectRef> {
        let name = match ctx {
            "Store" => "Store",
            "Del" => "Del",
            _ => "Load",
        };
        self.mk(name, vec![])
    }

    fn build_expr(&self, e: &Expr, ctx: &str) -> PyResult<PyObjectRef> {
        // Stamp source spans from `Expr::Located` (children stamp
        // themselves via their own recursive `build_expr` calls).
        let pos = e.position();
        let node = self.build_expr_inner(e.unlocate(), ctx)?;
        if let Some((line, col, end_line, end_col)) = pos {
            self.set_span(&node, line, col, end_line, end_col);
        }
        Ok(node)
    }

    fn build_expr_inner(&self, e: &Expr, ctx: &str) -> PyResult<PyObjectRef> {
        match e.unlocate() {
            Expr::Located { .. } => unreachable!("peeled via unlocate"),
            Expr::BoolOp { op, values } => {
                let on = match op {
                    BoolOp::And => "And",
                    BoolOp::Or => "Or",
                };
                self.mk(
                    "BoolOp",
                    vec![
                        ("op", self.mk(on, vec![])?),
                        ("values", self.build_expr_list(values, "Load")?),
                    ],
                )
            }
            Expr::NamedExpr { target, value } => self.mk(
                "NamedExpr",
                vec![
                    ("target", self.build_expr(target, "Store")?),
                    ("value", self.build_expr(value, "Load")?),
                ],
            ),
            Expr::BinOp { left, op, right } => self.mk(
                "BinOp",
                vec![
                    ("left", self.build_expr(left, "Load")?),
                    ("op", self.build_operator(op)?),
                    ("right", self.build_expr(right, "Load")?),
                ],
            ),
            Expr::UnaryOp { op, operand } => self.mk(
                "UnaryOp",
                vec![
                    ("op", self.build_unaryop(op)?),
                    ("operand", self.build_expr(operand, "Load")?),
                ],
            ),
            Expr::Lambda { args, body } => self.mk(
                "Lambda",
                vec![
                    ("args", self.build_arguments(args)?),
                    ("body", self.build_expr(body, "Load")?),
                ],
            ),
            Expr::IfExp { test, body, orelse } => self.mk(
                "IfExp",
                vec![
                    ("test", self.build_expr(test, "Load")?),
                    ("body", self.build_expr(body, "Load")?),
                    ("orelse", self.build_expr(orelse, "Load")?),
                ],
            ),
            Expr::Dict { keys, values } => {
                let ks: Vec<PyObjectRef> = keys
                    .iter()
                    .map(|k| match k {
                        Some(e) => self.build_expr(e, "Load"),
                        None => Ok(self.const_none()),
                    })
                    .collect::<PyResult<_>>()?;
                self.mk(
                    "Dict",
                    vec![
                        ("keys", py_list(ks)),
                        ("values", self.build_expr_list(values, "Load")?),
                    ],
                )
            }
            Expr::Set(elts) => self.mk("Set", vec![("elts", self.build_expr_list(elts, "Load")?)]),
            Expr::ListComp { elt, generators } => self.mk(
                "ListComp",
                vec![
                    ("elt", self.build_expr(elt, "Load")?),
                    ("generators", self.build_generators(generators)?),
                ],
            ),
            Expr::SetComp { elt, generators } => self.mk(
                "SetComp",
                vec![
                    ("elt", self.build_expr(elt, "Load")?),
                    ("generators", self.build_generators(generators)?),
                ],
            ),
            Expr::DictComp {
                key,
                value,
                generators,
            } => self.mk(
                "DictComp",
                vec![
                    ("key", self.build_expr(key, "Load")?),
                    ("value", self.build_expr(value, "Load")?),
                    ("generators", self.build_generators(generators)?),
                ],
            ),
            Expr::GeneratorExp { elt, generators } => self.mk(
                "GeneratorExp",
                vec![
                    ("elt", self.build_expr(elt, "Load")?),
                    ("generators", self.build_generators(generators)?),
                ],
            ),
            Expr::Await(v) => self.mk("Await", vec![("value", self.build_expr(v, "Load")?)]),
            Expr::Yield(v) => self.mk(
                "Yield",
                vec![(
                    "value",
                    match v {
                        Some(e) => self.build_expr(e, "Load")?,
                        None => self.const_none(),
                    },
                )],
            ),
            Expr::YieldFrom(v) => {
                self.mk("YieldFrom", vec![("value", self.build_expr(v, "Load")?)])
            }
            Expr::Compare {
                left,
                ops,
                comparators,
            } => self.mk(
                "Compare",
                vec![
                    ("left", self.build_expr(left, "Load")?),
                    (
                        "ops",
                        py_list(
                            ops.iter()
                                .map(|o| self.build_cmpop(o))
                                .collect::<PyResult<_>>()?,
                        ),
                    ),
                    ("comparators", self.build_expr_list(comparators, "Load")?),
                ],
            ),
            Expr::Call {
                func,
                args,
                keywords,
            } => self.mk(
                "Call",
                vec![
                    ("func", self.build_expr(func, "Load")?),
                    ("args", self.build_expr_list(args, "Load")?),
                    (
                        "keywords",
                        py_list(
                            keywords
                                .iter()
                                .map(|k| {
                                    self.mk(
                                        "keyword",
                                        vec![
                                            (
                                                "arg",
                                                match &k.arg {
                                                    Some(a) => py_str(a),
                                                    None => self.const_none(),
                                                },
                                            ),
                                            ("value", self.build_expr(&k.value, "Load")?),
                                        ],
                                    )
                                })
                                .collect::<PyResult<_>>()?,
                        ),
                    ),
                ],
            ),
            Expr::FString(parts) => {
                let mut values: Vec<PyObjectRef> = vec![];
                for p in parts {
                    match p {
                        FStringPart::String(s, span) => {
                            let node = self.mk(
                                "Constant",
                                vec![("value", py_str(s)), ("kind", self.const_none())],
                            )?;
                            let ((sl, sc), (el, ec)) = *span;
                            self.set_span(&node, sl, sc, el, ec);
                            values.push(node);
                        }
                        FStringPart::Expr {
                            expr,
                            conversion,
                            format_spec,
                            span,
                        } => {
                            let conv = match conversion {
                                0 => py_int(-1),
                                1 => py_int(114),
                                2 => py_int(115),
                                _ => py_int(97),
                            };
                            let fv = self.mk(
                                "FormattedValue",
                                vec![
                                    ("value", self.build_expr(expr, "Load")?),
                                    ("conversion", conv),
                                    (
                                        "format_spec",
                                        match format_spec {
                                            Some(f) => {
                                                // An explicit-but-empty spec
                                                // (`f'{a:}'`) is `JoinedStr`
                                                // with no values (mirrors
                                                // CPython), not None.
                                                let is_empty_str = matches!(
                                                    f.as_ref(),
                                                    Expr::Constant(
                                                        crate::ast::Constant::String(s)
                                                    ) if s.as_str().is_empty()
                                                );
                                                if is_empty_str {
                                                    self.mk(
                                                        "JoinedStr",
                                                        vec![("values", py_list(vec![]))],
                                                    )?
                                                } else {
                                                    let inner = self.build_expr(f, "Load")?;
                                                    // The spec expression may
                                                    // already be a JoinedStr
                                                    // (nested replacement
                                                    // fields parse as
                                                    // f-strings) — don't
                                                    // double-wrap.
                                                    let is_joined = match &*inner.borrow() {
                                                        PyObject::Instance { typ, .. } => {
                                                            match &*typ.borrow() {
                                                                PyObject::Type { name, .. } => {
                                                                    name == "JoinedStr"
                                                                }
                                                                _ => false,
                                                            }
                                                        }
                                                        _ => false,
                                                    };
                                                    if is_joined {
                                                        inner
                                                    } else {
                                                        self.mk(
                                                            "JoinedStr",
                                                            vec![("values", py_list(vec![inner]))],
                                                        )?
                                                    }
                                                }
                                            }
                                            None => self.const_none(),
                                        },
                                    ),
                                ],
                            )?;
                            let ((fsl, fsc), (fel, fec)) = *span;
                            self.set_span(&fv, fsl, fsc, fel, fec);
                            values.push(fv);
                        }
                    }
                }
                self.mk("JoinedStr", vec![("values", py_list(values))])
            }
            Expr::TemplateStr(parts) => {
                let mut values: Vec<PyObjectRef> = vec![];
                for p in parts {
                    match p {
                        TemplateStrPart::String(s, span) => {
                            let node = self.mk(
                                "Constant",
                                vec![("value", py_str(s)), ("kind", self.const_none())],
                            )?;
                            let ((sl, sc), (el, ec)) = *span;
                            self.set_span(&node, sl, sc, el, ec);
                            values.push(node);
                        }
                        TemplateStrPart::Interpolation {
                            expr,
                            str_text,
                            conversion,
                            format_spec,
                            span,
                        } => {
                            let conv = match conversion {
                                0 => py_int(-1),
                                1 => py_int(114),
                                2 => py_int(115),
                                _ => py_int(97),
                            };
                            let interp = self.mk(
                                "Interpolation",
                                vec![
                                    ("value", self.build_expr(expr, "Load")?),
                                    ("str", py_str(str_text)),
                                    ("conversion", conv),
                                    (
                                        "format_spec",
                                        match format_spec {
                                            Some(f) => {
                                                let inner = self.build_expr(f, "Load")?;
                                                let is_joined = match &*inner.borrow() {
                                                    PyObject::Instance { typ, .. } => {
                                                        match &*typ.borrow() {
                                                            PyObject::Type { name, .. } => {
                                                                name == "JoinedStr"
                                                            }
                                                            _ => false,
                                                        }
                                                    }
                                                    _ => false,
                                                };
                                                if is_joined {
                                                    inner
                                                } else {
                                                    self.mk(
                                                        "JoinedStr",
                                                        vec![("values", py_list(vec![inner]))],
                                                    )?
                                                }
                                            }
                                            None => self.const_none(),
                                        },
                                    ),
                                ],
                            )?;
                            let ((fsl, fsc), (fel, fec)) = *span;
                            self.set_span(&interp, fsl, fsc, fel, fec);
                            values.push(interp);
                        }
                    }
                }
                self.mk("TemplateStr", vec![("values", py_list(values))])
            }
            Expr::JoinedStr(_) => self.mk("JoinedStr", vec![("values", py_list(vec![]))]),
            Expr::Constant(c) => self.mk(
                "Constant",
                vec![
                    ("value", self.build_constant(c)?),
                    ("kind", self.const_none()),
                ],
            ),
            Expr::Attribute { value, attr } => self.mk(
                "Attribute",
                vec![
                    ("value", self.build_expr(value, "Load")?),
                    ("attr", py_str(attr)),
                    ("ctx", self.build_ctx(ctx)?),
                ],
            ),
            Expr::Subscript { value, slice } => {
                let sl = self.build_expr(slice, "Load")?;
                // CPython wraps a bare starred slice in a Tuple:
                // `tuple[*Ts]` has slice=Tuple(elts=[Starred]), never a
                // bare Starred.
                let is_starred = match &*sl.borrow() {
                    PyObject::Instance { typ, .. } => match &*typ.borrow() {
                        PyObject::Type { name, .. } => name == "Starred",
                        _ => false,
                    },
                    _ => false,
                };
                let sl = if is_starred {
                    self.mk(
                        "Tuple",
                        vec![
                            ("elts", py_list(vec![sl])),
                            ("ctx", self.build_ctx("Load")?),
                        ],
                    )?
                } else {
                    sl
                };
                self.mk(
                    "Subscript",
                    vec![
                        ("value", self.build_expr(value, "Load")?),
                        ("slice", sl),
                        ("ctx", self.build_ctx(ctx)?),
                    ],
                )
            }
            Expr::Starred(e) => self.mk(
                "Starred",
                vec![
                    ("value", self.build_expr(e, ctx)?),
                    ("ctx", self.build_ctx(ctx)?),
                ],
            ),
            Expr::Name(id) if self.optimize_ast && id == "__debug__" => {
                // `PyCF_OPTIMIZED_AST`: `__debug__` is folded to a
                // `Constant` holding its CURRENT value.
                let current = crate::vm::get_shared_builtins_module()
                    .borrow()
                    .get_attribute("__debug__")
                    .unwrap_or_else(|_| crate::object::py_bool(true));
                let value = match &*current.borrow() {
                    crate::object::PyObject::Bool(b) => crate::object::py_bool(*b),
                    _ => crate::object::py_bool(true),
                };
                self.mk("Constant", vec![("value", value), ("kind", py_none())])
            }
            Expr::Name(id) => self.mk(
                "Name",
                vec![("id", py_str(id)), ("ctx", self.build_ctx(ctx)?)],
            ),
            Expr::List(elts) => self.mk(
                "List",
                vec![
                    ("elts", self.build_expr_list(elts, ctx)?),
                    ("ctx", self.build_ctx(ctx)?),
                ],
            ),
            Expr::Tuple(elts) => self.mk(
                "Tuple",
                vec![
                    ("elts", self.build_expr_list(elts, ctx)?),
                    ("ctx", self.build_ctx(ctx)?),
                ],
            ),
            Expr::Slice { lower, upper, step } => self.mk(
                "Slice",
                vec![
                    (
                        "lower",
                        match lower {
                            Some(e) => self.build_expr(e, "Load")?,
                            None => self.const_none(),
                        },
                    ),
                    (
                        "upper",
                        match upper {
                            Some(e) => self.build_expr(e, "Load")?,
                            None => self.const_none(),
                        },
                    ),
                    (
                        "step",
                        match step {
                            Some(e) => self.build_expr(e, "Load")?,
                            None => self.const_none(),
                        },
                    ),
                ],
            ),
        }
    }

    fn build_generators(&self, gens: &[Comprehension]) -> PyResult<PyObjectRef> {
        Ok(py_list(
            gens.iter()
                .map(|g| {
                    self.mk(
                        "comprehension",
                        vec![
                            ("target", self.build_expr(&g.target, "Store")?),
                            ("iter", self.build_expr(&g.iter, "Load")?),
                            ("ifs", self.build_expr_list(&g.ifs, "Load")?),
                            ("is_async", py_int(g.is_async as i64)),
                        ],
                    )
                })
                .collect::<PyResult<_>>()?,
        ))
    }

    fn build_constant(&self, c: &Constant) -> PyResult<PyObjectRef> {
        match c {
            Constant::None => Ok(py_none()),
            Constant::Bool(b) => Ok(py_bool(*b)),
            Constant::Int(s) => Ok(parse_int_literal(s)),
            Constant::Float(s) => {
                let clean: String = s.chars().filter(|&ch| ch != '_').collect();
                match clean.parse::<f64>() {
                    Ok(v) => Ok(py_float(v)),
                    Err(_) => Err(PyError::value_error(format!("invalid float literal {}", s))),
                }
            }
            Constant::Complex { real, imag } => {
                let re: f64 = real.replace('_', "").parse().unwrap_or(0.0);
                let im: f64 = imag.replace('_', "").parse().unwrap_or(0.0);
                Ok(PyObjectRef::new(PyObject::Complex(re, im)))
            }
            Constant::String(s) => Ok(py_str(s.as_str())),
            Constant::Bytes(b) => Ok(PyObjectRef::new(PyObject::Bytes(b.clone()))),
            Constant::Ellipsis => {
                let bmod = crate::vm::util::get_shared_builtins_module();
                let e = bmod
                    .borrow()
                    .get_attribute("Ellipsis")
                    .map_err(|_| PyError::runtime_error("Ellipsis builtin missing".to_string()))?;
                Ok(e)
            }
        }
    }

    fn build_pattern(&self, p: &Pattern) -> PyResult<PyObjectRef> {
        match p {
            Pattern::MatchValue(e) => {
                let v = self.build_expr(e, "Load")?;
                self.mk("MatchValue", vec![("value", v)])
            }
            Pattern::MatchSingleton(s) => {
                let v = match s.as_str() {
                    "None" => py_none(),
                    "True" => py_bool(true),
                    "False" => py_bool(false),
                    _ => py_str(s),
                };
                self.mk("MatchSingleton", vec![("value", v)])
            }
            Pattern::MatchSequence(pats) => self.mk(
                "MatchSequence",
                vec![(
                    "patterns",
                    py_list(
                        pats.iter()
                            .map(|q| self.build_pattern(q))
                            .collect::<PyResult<_>>()?,
                    ),
                )],
            ),
            Pattern::MatchMapping { keys, rest } => {
                // Interleaved [key_pat, val_pat, ...]: split into parallel
                // keys (inner exprs) and patterns.
                let mut ks: Vec<PyObjectRef> = vec![];
                let mut ps: Vec<PyObjectRef> = vec![];
                let mut it = keys.iter();
                while let Some(kp) = it.next() {
                    let ke = match kp {
                        Pattern::MatchValue(e) => self.build_expr(e, "Load")?,
                        _ => self.build_pattern(kp)?,
                    };
                    ks.push(ke);
                    if let Some(vp) = it.next() {
                        ps.push(self.build_pattern(vp)?);
                    }
                }
                self.mk(
                    "MatchMapping",
                    vec![
                        ("keys", py_list(ks)),
                        ("patterns", py_list(ps)),
                        (
                            "rest",
                            match rest {
                                Some(n) => py_str(n),
                                None => self.const_none(),
                            },
                        ),
                    ],
                )
            }
            Pattern::MatchClass {
                cls,
                patterns,
                kwd_attrs,
                kwd_patterns,
            } => self.mk(
                "MatchClass",
                vec![
                    ("cls", self.build_expr(cls, "Load")?),
                    (
                        "patterns",
                        py_list(
                            patterns
                                .iter()
                                .map(|q| self.build_pattern(q))
                                .collect::<PyResult<_>>()?,
                        ),
                    ),
                    (
                        "kwd_attrs",
                        py_list(kwd_attrs.iter().map(|a| py_str(a)).collect()),
                    ),
                    (
                        "kwd_patterns",
                        py_list(
                            kwd_patterns
                                .iter()
                                .map(|q| self.build_pattern(q))
                                .collect::<PyResult<_>>()?,
                        ),
                    ),
                ],
            ),
            Pattern::MatchStar { name } => self.mk(
                "MatchStar",
                vec![(
                    "name",
                    match name {
                        Some(n) => py_str(n),
                        None => self.const_none(),
                    },
                )],
            ),
            Pattern::MatchAs { pattern, name } => self.mk(
                "MatchAs",
                vec![
                    (
                        "pattern",
                        match pattern {
                            Some(q) => self.build_pattern(q)?,
                            None => self.const_none(),
                        },
                    ),
                    (
                        "name",
                        match name {
                            Some(n) => py_str(n),
                            None => self.const_none(),
                        },
                    ),
                ],
            ),
            Pattern::MatchOr(pats) => self.mk(
                "MatchOr",
                vec![(
                    "patterns",
                    py_list(
                        pats.iter()
                            .map(|q| self.build_pattern(q))
                            .collect::<PyResult<_>>()?,
                    ),
                )],
            ),
        }
    }
}

/// Parse `"(args) -> returns"` (mode="func_type") into FunctionType.
/// Splits on the top-level `->`, parses each side as expressions.
fn parse_func_type(source: &str, filename: &str) -> PyResult<PyObjectRef> {
    let err = |msg: String| PyError::syntax_error_with_filename(msg, filename, source);
    // Find top-level `->` (respecting bracket nesting).
    let chars: Vec<char> = source.chars().collect();
    let mut depth = 0;
    let mut arrow = None;
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            '-' if depth == 0 && chars.get(i + 1) == Some(&'>') => {
                arrow = Some(i);
                break;
            }
            _ => {}
        }
        i += 1;
    }
    let ai = arrow.ok_or_else(|| err("invalid syntax".to_string()))?;
    let args_src: String = chars[..ai].iter().collect();
    let ret_src: String = chars[ai + 2..].iter().collect();
    let mut ap = crate::parser::Parser::new(args_src.trim(), true);
    ap.allow_top_level_await = true;
    let args_expr = ap.parse_single_expr().map_err(|m| err(m))?;
    let mut rp = crate::parser::Parser::new(ret_src.trim(), true);
    rp.allow_top_level_await = true;
    let ret_expr = rp.parse_single_expr().map_err(|m| err(m))?;
    let mut b = AstBuilder::new()?;
    // Args must be a parenthesized tuple (possibly empty).
    // Peel positions (see `Expr::Located`).
    let argtypes: Vec<PyObjectRef> = match args_expr.unlocate() {
        Expr::Tuple(elts) => elts
            .iter()
            .map(|e| b.build_expr(e, "Load"))
            .collect::<PyResult<_>>()?,
        _ => {
            return Err(err(
                "invalid syntax: func_type args must be parenthesized".to_string()
            ));
        }
    };
    let returns = b.build_expr(&ret_expr, "Load")?;
    b.mk(
        "FunctionType",
        vec![("argtypes", py_list(argtypes)), ("returns", returns)],
    )
}

fn parse_int_literal(s: &str) -> PyObjectRef {
    let clean: String = s.chars().filter(|&c| c != '_').collect();
    let (radix, digits) = if let Some(h) = clean
        .strip_prefix("0x")
        .or_else(|| clean.strip_prefix("0X"))
    {
        (16, h)
    } else if let Some(o) = clean
        .strip_prefix("0o")
        .or_else(|| clean.strip_prefix("0O"))
    {
        (8, o)
    } else if let Some(b) = clean
        .strip_prefix("0b")
        .or_else(|| clean.strip_prefix("0B"))
    {
        (2, b)
    } else {
        (10, clean.as_str())
    };
    match num_bigint::BigInt::parse_bytes(digits.as_bytes(), radix) {
        Some(n) => py_int(n),
        None => py_int(0),
    }
}

/// Entry point for `_ast_native._parse_tree(source, filename, mode)`.
pub fn parse_tree(source: &str, filename: &str, mode: &str) -> PyResult<PyObjectRef> {
    parse_tree_with_optimize(source, filename, mode, false)
}

/// `parse_tree`, with `ast.PyCF_OPTIMIZED_AST` support: the optimizer
/// folds `__debug__` (see `AstBuilder::with_optimize`).
pub fn parse_tree_with_optimize(
    source: &str,
    filename: &str,
    mode: &str,
    optimize_ast: bool,
) -> PyResult<PyObjectRef> {
    let mut parser = crate::parser::Parser::new(source, mode == "eval");
    // `ast.parse` builds a tree without compiling: async constructs are
    // legal anywhere at parse time (CPython validates async context at
    // compile time, not parse time).
    parser.allow_top_level_await = true;
    if mode == "func_type" {
        return parse_func_type(source, filename);
    }
    if mode == "eval" {
        // Expression-only: parse a single expression directly.
        match parser.parse_single_expr() {
            Ok(e) => {
                let mut b = AstBuilder::with_optimize(optimize_ast)?;
                let body = b.build_expr(&e, "Load")?;
                return b.mk("Expression", vec![("body", body)]);
            }
            Err(msg) => {
                return Err(PyError::syntax_error_with_filename(msg, filename, source));
            }
        }
    }
    match parser.parse_program() {
        Ok(prog) => {
            let mut b = AstBuilder::with_optimize(optimize_ast)?;
            b.build_program(&prog, mode)
        }
        Err(msg) => Err(PyError::syntax_error_with_filename(msg, filename, source)),
    }
}
