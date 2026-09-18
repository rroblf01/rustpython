use std::collections::HashSet;

use crate::ast::*;

use super::Parser;

/// Per-function names seen so far while parsing its body, used to reject
/// `global`/`nonlocal` after a prior use/assign/annot in the same scope
/// (CPython `symtable.c`: `Global_kind` checks `DEF_PARAM | DEF_LOCAL |
/// `USE | DEF_ANNOT` in order). Params are already tracked separately
/// (`fn_params_stack`); this covers the other three.
#[derive(Default)]
pub(crate) struct FnScopeTracker {
    pub assigned: HashSet<String>,
    pub used: HashSet<String>,
    pub annotated: HashSet<String>,
    pub globals: HashSet<String>,
    pub nonlocals: HashSet<String>,
    /// `class_depth` when this function was entered: names inside a class
    /// body nested in the function belong to the class scope, not this
    /// function's (`def f(): class C: x = 1` must not poison `global x`
    /// inside `C`, and vice versa).
    pub class_depth_at_entry: usize,
}

impl Parser {
    /// Record a just-parsed statement's bindings/uses into the innermost
    /// function scope under construction, if any. Called from every site
    /// that pushes a `Stmt::Located` (parse_program is module-level, so
    /// the stack is empty there and this is a no-op).
    pub(crate) fn push_tracked_stmt(&mut self, stmts: &mut Vec<Stmt>, line: usize, stmt: Stmt) {
        self.track_stmt_names(&stmt);
        stmts.push(Stmt::Located(line, Box::new(stmt)));
    }

    fn active_tracker(&mut self) -> Option<&mut FnScopeTracker> {
        // Inside a class body nested in the function, names belong to the
        // class scope — same reason `parse_global`'s param check skips
        // when `class_depth > 0`, but relative to function entry so that
        // methods (`class C: def f(): ...`) are still validated.
        let class_depth = self.class_depth;
        self.fn_scope_stack
            .last_mut()
            .filter(|tr| class_depth <= tr.class_depth_at_entry)
    }

    fn track_stmt_names(&mut self, stmt: &Stmt) {
        // Borrow dance: walk first with a scratch pass that only needs
        // &self for nothing — the walkers are free functions below.
        let mut assigned = Vec::new();
        let mut used = Vec::new();
        let mut annotated = Vec::new();
        let mut declared_global = Vec::new();
        let mut declared_nonlocal = Vec::new();
        collect_stmt_names(
            stmt,
            &mut assigned,
            &mut used,
            &mut annotated,
            &mut declared_global,
            &mut declared_nonlocal,
        );
        if let Some(tr) = self.active_tracker() {
            tr.assigned.extend(assigned);
            tr.used.extend(used);
            tr.annotated.extend(annotated);
            tr.globals.extend(declared_global);
            tr.nonlocals.extend(declared_nonlocal);
        }
    }

    /// Validate `global names` at (line, col) against prior activity in the
    /// current function scope. Param conflicts are handled by the caller
    /// (`parse_global`); this covers use/annot/assign + global/nonlocal
    /// collisions, in CPython's check order (USE, ANNOT, LOCAL).
    pub(crate) fn check_global_order(
        &mut self,
        names: &[String],
        line: usize,
        col: usize,
    ) -> Result<(), String> {
        let entry_ok = self
            .fn_scope_stack
            .last()
            .is_some_and(|tr| self.class_depth <= tr.class_depth_at_entry);
        if !entry_ok {
            return Ok(());
        }
        let tr = self.fn_scope_stack.last().unwrap();
        for n in names {
            if tr.nonlocals.contains(n) {
                return Err(format!("L{line}:{col}: name '{n}' is nonlocal and global"));
            } else if tr.used.contains(n) {
                return Err(format!(
                    "L{line}:{col}: name '{n}' is used prior to global declaration"
                ));
            } else if tr.annotated.contains(n) {
                return Err(format!(
                    "L{line}:{col}: annotated name '{n}' can't be global"
                ));
            } else if tr.assigned.contains(n) {
                return Err(format!(
                    "L{line}:{col}: name '{n}' is assigned to before global declaration"
                ));
            }
        }
        Ok(())
    }

    pub(crate) fn check_nonlocal_order(
        &mut self,
        names: &[String],
        line: usize,
        col: usize,
    ) -> Result<(), String> {
        let entry_ok = self
            .fn_scope_stack
            .last()
            .is_some_and(|tr| self.class_depth <= tr.class_depth_at_entry);
        if !entry_ok {
            return Ok(());
        }
        let tr = self.fn_scope_stack.last().unwrap();
        for n in names {
            if tr.globals.contains(n) {
                return Err(format!("L{line}:{col}: name '{n}' is nonlocal and global"));
            } else if tr.used.contains(n) {
                return Err(format!(
                    "L{line}:{col}: name '{n}' is used prior to nonlocal declaration"
                ));
            } else if tr.annotated.contains(n) {
                return Err(format!(
                    "L{line}:{col}: annotated name '{n}' can't be nonlocal"
                ));
            } else if tr.assigned.contains(n) {
                return Err(format!(
                    "L{line}:{col}: name '{n}' is assigned to before nonlocal declaration"
                ));
            }
        }
        Ok(())
    }
}

/// Walk one statement, sorting names into assigned/used/annotated buckets.
/// Never descends into nested `def`/`class` bodies (separate scopes, parsed
/// under their own tracker); control-flow bodies (`if`/`for`/...) are the
/// same scope and are walked. Match-pattern and comprehension bindings are
/// skipped — same limitation as the compiler's own `collect_assigned_inner`.
#[allow(clippy::too_many_arguments)]
fn collect_stmt_names(
    stmt: &Stmt,
    assigned: &mut Vec<String>,
    used: &mut Vec<String>,
    annotated: &mut Vec<String>,
    declared_global: &mut Vec<String>,
    declared_nonlocal: &mut Vec<String>,
) {
    match stmt {
        Stmt::Assign { targets, value } => {
            collect_expr_uses(value, used);
            for t in targets {
                collect_target_names(t, assigned, used);
            }
        }
        Stmt::AugAssign { target, value, .. } => {
            // `x += 1` both reads and rebinds `x`.
            collect_target_names(target, assigned, used);
            collect_expr_uses(target, used);
            collect_expr_uses(value, used);
        }
        Stmt::AnnAssign {
            target,
            annotation,
            value,
        } => {
            collect_expr_uses(annotation, used);
            if let Some(v) = value {
                collect_expr_uses(v, used);
                collect_target_names(target, assigned, used);
            }
            collect_ann_target_names(target, annotated);
        }
        Stmt::Return(Some(e)) | Stmt::Expr(e) => collect_expr_uses(e, used),
        Stmt::Return(None) | Stmt::Pass | Stmt::Break | Stmt::Continue => {}
        Stmt::Delete(targets) => {
            for t in targets {
                collect_target_names(t, assigned, used);
            }
        }
        Stmt::If { test, body, orelse } => {
            collect_expr_uses(test, used);
            collect_block_names(
                body,
                assigned,
                used,
                annotated,
                declared_global,
                declared_nonlocal,
            );
            collect_block_names(
                orelse,
                assigned,
                used,
                annotated,
                declared_global,
                declared_nonlocal,
            );
        }
        Stmt::While { test, body, orelse } => {
            collect_expr_uses(test, used);
            collect_block_names(
                body,
                assigned,
                used,
                annotated,
                declared_global,
                declared_nonlocal,
            );
            collect_block_names(
                orelse,
                assigned,
                used,
                annotated,
                declared_global,
                declared_nonlocal,
            );
        }
        Stmt::For {
            target,
            iter,
            body,
            orelse,
            ..
        } => {
            collect_expr_uses(iter, used);
            collect_target_names(target, assigned, used);
            collect_block_names(
                body,
                assigned,
                used,
                annotated,
                declared_global,
                declared_nonlocal,
            );
            collect_block_names(
                orelse,
                assigned,
                used,
                annotated,
                declared_global,
                declared_nonlocal,
            );
        }
        Stmt::With { items, body, .. } => {
            for item in items {
                collect_expr_uses(&item.context_expr, used);
                if let Some(var) = &item.optional_vars {
                    collect_target_names(var, assigned, used);
                }
            }
            collect_block_names(
                body,
                assigned,
                used,
                annotated,
                declared_global,
                declared_nonlocal,
            );
        }
        Stmt::Match { subject, cases } => {
            collect_expr_uses(subject, used);
            for case in cases {
                collect_pattern_names(&case.pattern, assigned, used);
                if let Some(g) = &case.guard {
                    collect_expr_uses(g, used);
                }
                collect_block_names(
                    &case.body,
                    assigned,
                    used,
                    annotated,
                    declared_global,
                    declared_nonlocal,
                );
            }
        }
        Stmt::Try {
            body,
            handlers,
            handlers_star,
            orelse,
            finalbody,
        } => {
            collect_block_names(
                body,
                assigned,
                used,
                annotated,
                declared_global,
                declared_nonlocal,
            );
            for h in handlers {
                if let Some(t) = &h.typ {
                    collect_expr_uses(t, used);
                }
                if let Some(n) = &h.name {
                    assigned.push(n.clone());
                }
                collect_block_names(
                    &h.body,
                    assigned,
                    used,
                    annotated,
                    declared_global,
                    declared_nonlocal,
                );
            }
            for h in handlers_star {
                if let Some(t) = &h.typ {
                    collect_expr_uses(t, used);
                }
                if let Some(n) = &h.name {
                    assigned.push(n.clone());
                }
                collect_block_names(
                    &h.body,
                    assigned,
                    used,
                    annotated,
                    declared_global,
                    declared_nonlocal,
                );
            }
            collect_block_names(
                orelse,
                assigned,
                used,
                annotated,
                declared_global,
                declared_nonlocal,
            );
            collect_block_names(
                finalbody,
                assigned,
                used,
                annotated,
                declared_global,
                declared_nonlocal,
            );
        }
        Stmt::Raise { exc, cause } => {
            if let Some(e) = exc {
                collect_expr_uses(e, used);
            }
            if let Some(c) = cause {
                collect_expr_uses(c, used);
            }
        }
        Stmt::Assert { test, msg } => {
            collect_expr_uses(test, used);
            if let Some(m) = msg {
                collect_expr_uses(m, used);
            }
        }
        Stmt::FunctionDef {
            name,
            decorator_list,
            args,
            returns,
            ..
        } => {
            // Evaluated in the enclosing scope; the body is separate.
            for d in decorator_list {
                collect_expr_uses(d, used);
            }
            for a in args {
                if let Some(d) = &a.default {
                    collect_expr_uses(d, used);
                }
                if let Some(ann) = &a.annotation {
                    collect_expr_uses(ann, used);
                }
            }
            if let Some(r) = returns {
                collect_expr_uses(r, used);
            }
            assigned.push(name.clone());
        }
        Stmt::ClassDef {
            name,
            bases,
            keywords,
            decorator_list,
            ..
        } => {
            for d in decorator_list {
                collect_expr_uses(d, used);
            }
            for b in bases {
                collect_expr_uses(b, used);
            }
            for k in keywords {
                collect_expr_uses(&k.value, used);
            }
            assigned.push(name.clone());
        }
        Stmt::Import(list) => {
            for a in list {
                assigned.push(a.asname.clone().unwrap_or_else(|| a.name.clone()));
            }
        }
        Stmt::ImportFrom { names, .. } => {
            for a in names {
                assigned.push(a.asname.clone().unwrap_or_else(|| a.name.clone()));
            }
        }
        Stmt::Global(names) => declared_global.extend(names.iter().cloned()),
        Stmt::Nonlocal(names) => declared_nonlocal.extend(names.iter().cloned()),
        Stmt::TypeAlias { name, value, .. } => {
            collect_expr_uses(value, used);
            assigned.push(name.clone());
        }
        Stmt::Located(_, inner) => collect_stmt_names(
            inner,
            assigned,
            used,
            annotated,
            declared_global,
            declared_nonlocal,
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn collect_block_names(
    stmts: &[Stmt],
    assigned: &mut Vec<String>,
    used: &mut Vec<String>,
    annotated: &mut Vec<String>,
    declared_global: &mut Vec<String>,
    declared_nonlocal: &mut Vec<String>,
) {
    for s in stmts {
        collect_stmt_names(
            s,
            assigned,
            used,
            annotated,
            declared_global,
            declared_nonlocal,
        );
    }
}

/// Names read (value/class expressions) vs bound (captures) by a
/// match-case pattern. Captures are assignments in the containing scope
/// (so `global x` after `case [x]` is an error, like any late global);
/// value expressions are ordinary reads.
fn collect_pattern_names(pat: &Pattern, assigned: &mut Vec<String>, used: &mut Vec<String>) {
    match pat {
        Pattern::MatchValue(e) => collect_expr_uses(e, used),
        Pattern::MatchClass {
            cls,
            patterns,
            kwd_patterns,
            ..
        } => {
            collect_expr_uses(cls, used);
            for p in patterns.iter().chain(kwd_patterns.iter()) {
                collect_pattern_names(p, assigned, used);
            }
        }
        Pattern::MatchSequence(pats) | Pattern::MatchOr(pats) => {
            for p in pats {
                collect_pattern_names(p, assigned, used);
            }
        }
        Pattern::MatchMapping { keys, rest } => {
            for k in keys {
                collect_pattern_names(k, assigned, used);
            }
            if let Some(n) = rest {
                assigned.push(n.clone());
            }
        }
        Pattern::MatchStar { name } => {
            if let Some(n) = name {
                assigned.push(n.clone());
            }
        }
        Pattern::MatchAs { pattern, name } => {
            if let Some(n) = name {
                assigned.push(n.clone());
            }
            if let Some(inner) = pattern {
                collect_pattern_names(inner, assigned, used);
            }
        }
        Pattern::MatchSingleton(_) => {}
    }
}

/// Bind targets: plain names bind; tuple/list/starred destructure; anything
/// else (`obj.attr`, `obj[i]`) reads its sub-expressions.
fn collect_target_names(target: &Expr, assigned: &mut Vec<String>, used: &mut Vec<String>) {
    match target {
        Expr::Name(n) => assigned.push(n.clone()),
        Expr::List(elts) | Expr::Tuple(elts) => {
            for e in elts {
                collect_target_names(e, assigned, used);
            }
        }
        Expr::Starred(e) => collect_target_names(e, assigned, used),
        _ => collect_expr_uses(target, used),
    }
}

/// `x: Ann` (with or without `= value`) marks `x` annotated.
fn collect_ann_target_names(target: &Expr, annotated: &mut Vec<String>) {
    match target {
        Expr::Name(n) => annotated.push(n.clone()),
        Expr::List(elts) | Expr::Tuple(elts) => {
            for e in elts {
                collect_ann_target_names(e, annotated);
            }
        }
        Expr::Starred(e) => collect_ann_target_names(e, annotated),
        _ => {}
    }
}

/// All `Name` loads in an expression. Skips nested scopes (`lambda` bodies,
/// comprehensions); lambda/comprehension defaults are also skipped — a
/// documented over-approximation gap with no test coverage.
fn collect_expr_uses(expr: &Expr, used: &mut Vec<String>) {
    match expr {
        Expr::Name(n) => used.push(n.clone()),
        Expr::Lambda { .. }
        | Expr::ListComp { .. }
        | Expr::SetComp { .. }
        | Expr::DictComp { .. }
        | Expr::GeneratorExp { .. } => {}
        Expr::BoolOp { values, .. } => {
            for v in values {
                collect_expr_uses(v, used);
            }
        }
        Expr::NamedExpr { target, value } => {
            // Walrus also binds, but this walker only tracks uses;
            // recording the target as used still triggers the
            // use-before-global error (same SyntaxError, adjacent
            // message). No test covers walrus-then-global.
            collect_expr_uses(target, used);
            collect_expr_uses(value, used);
        }
        Expr::BinOp { left, right, .. } => {
            collect_expr_uses(left, used);
            collect_expr_uses(right, used);
        }
        Expr::UnaryOp { operand, .. } => collect_expr_uses(operand, used),
        Expr::IfExp { test, body, orelse } => {
            collect_expr_uses(test, used);
            collect_expr_uses(body, used);
            collect_expr_uses(orelse, used);
        }
        Expr::Dict { keys, values } => {
            for k in keys.iter().flatten() {
                collect_expr_uses(k, used);
            }
            for v in values {
                collect_expr_uses(v, used);
            }
        }
        Expr::Set(elts) | Expr::List(elts) | Expr::Tuple(elts) => {
            for e in elts {
                collect_expr_uses(e, used);
            }
        }
        Expr::Await(e) | Expr::YieldFrom(e) | Expr::Starred(e) => collect_expr_uses(e, used),
        Expr::Yield(Some(e)) => collect_expr_uses(e, used),
        Expr::Yield(None) => {}
        Expr::Compare {
            left, comparators, ..
        } => {
            collect_expr_uses(left, used);
            for c in comparators {
                collect_expr_uses(c, used);
            }
        }
        Expr::Call {
            func,
            args,
            keywords,
        } => {
            collect_expr_uses(func, used);
            for a in args {
                collect_expr_uses(a, used);
            }
            for k in keywords {
                collect_expr_uses(&k.value, used);
            }
        }
        Expr::FString(parts) => {
            for p in parts {
                if let FStringPart::Expr {
                    expr, format_spec, ..
                } = p
                {
                    collect_expr_uses(expr, used);
                    if let Some(fs) = format_spec {
                        collect_expr_uses(fs, used);
                    }
                }
            }
        }
        Expr::JoinedStr(elts) => {
            for e in elts {
                collect_expr_uses(e, used);
            }
        }
        Expr::Constant(_) => {}
        Expr::Attribute { value, .. } => collect_expr_uses(value, used),
        Expr::Subscript { value, slice } => {
            collect_expr_uses(value, used);
            collect_expr_uses(slice, used);
        }
        Expr::Slice { lower, upper, step } => {
            if let Some(s) = lower {
                collect_expr_uses(s, used);
            }
            if let Some(s) = upper {
                collect_expr_uses(s, used);
            }
            if let Some(s) = step {
                collect_expr_uses(s, used);
            }
        }
    }
}
