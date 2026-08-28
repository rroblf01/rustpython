use super::Compiler;
use crate::ast::*;

// ---- Free helper functions (extracted from compiler.rs) ----

pub fn contains_yield_in_stmts(stmts: &[Stmt]) -> bool {
    stmts.iter().any(|s| match Compiler::unwrap_located(s) {
        Stmt::Expr(expr)
        | Stmt::Return(Some(expr))
        | Stmt::Assign { value: expr, .. }
        | Stmt::AugAssign { value: expr, .. } => contains_yield_in_expr(expr),
        Stmt::If { test, body, orelse } => {
            contains_yield_in_expr(test)
                || contains_yield_in_stmts(body)
                || contains_yield_in_stmts(orelse)
        }
        Stmt::While { test, body, orelse } => {
            contains_yield_in_expr(test)
                || contains_yield_in_stmts(body)
                || contains_yield_in_stmts(orelse)
        }
        Stmt::For {
            iter, body, orelse, ..
        } => {
            contains_yield_in_expr(iter)
                || contains_yield_in_stmts(body)
                || contains_yield_in_stmts(orelse)
        }
        Stmt::With { items, body, .. } => {
            items
                .iter()
                .any(|i| contains_yield_in_expr(&i.context_expr))
                || contains_yield_in_stmts(body)
        }
        Stmt::Try {
            body,
            handlers,
            handlers_star,
            orelse,
            finalbody,
        } => {
            contains_yield_in_stmts(body)
                || handlers.iter().any(|h| contains_yield_in_stmts(&h.body))
                || handlers_star
                    .iter()
                    .any(|h| contains_yield_in_stmts(&h.body))
                || contains_yield_in_stmts(orelse)
                || contains_yield_in_stmts(finalbody)
        }
        // A nested `def`/`async def`/`class` starts its own independent
        // scope — whether *it* contains `yield`/`await` has no bearing on
        // whether the *enclosing* function is a generator/coroutine. This
        // used to recurse into the nested body, so e.g. a plain nested
        // helper `def decorator(func): ... async def wrapper(...): return
        // await func(...) ... return wrapper` (real code:
        // `django.utils.deprecation.deprecate_posargs`, an ordinary
        // sync/async-dispatching decorator factory, no yield/await
        // anywhere in its own body) made every *enclosing* function
        // wrongly compiled as a generator too — calling it returned a bare
        // generator object instead of ever running its body, since nothing
        // actually executes until the generator is iterated. Confirmed
        // minimal repro: a function returning a nested function containing
        // only a conditionally-defined `async def` sibling came back as
        // `<generator object>` instead of the callable it should return.
        Stmt::FunctionDef { .. } | Stmt::ClassDef { .. } => false,
        _ => false,
    })
}

/// Top-level `await`/`async for`/`async with` (only reachable when compiled
/// with PyCF_ALLOW_TOP_LEVEL_AWAIT) marks a module as a coroutine.
/// CPython's per-target `del` error messages (test_syntax::test_assign_del).
pub fn delete_error_for(expr: &Expr) -> &'static str {
    match expr {
        Expr::Name(_) => "cannot delete 'name'",
        Expr::Constant(_) => "cannot delete literal",
        Expr::Call { .. } => "cannot delete function call",
        Expr::Starred(_) => "cannot delete starred",
        Expr::NamedExpr { .. } => "cannot delete named expression",
        Expr::IfExp { .. } => "cannot delete conditional",
        Expr::BinOp { .. } | Expr::UnaryOp { .. } => "cannot delete expression",
        Expr::BoolOp { .. } | Expr::Compare { .. } => "cannot delete expression",
        _ => "cannot delete expression",
    }
}

pub fn stmt_has_top_level_await(stmt: &Stmt) -> bool {
    match Compiler::unwrap_located(stmt) {
        Stmt::For { is_async: true, .. } | Stmt::With { is_async: true, .. } => true,
        _ => contains_yield_in_stmts(std::slice::from_ref(stmt)),
    }
}

pub fn contains_yield_in_expr(expr: &Expr) -> bool {
    match expr {
        Expr::Yield(_) => true,
        Expr::YieldFrom(_) => true,
        Expr::Await(_) => true,
        Expr::BinOp { left, right, .. } => {
            contains_yield_in_expr(left) || contains_yield_in_expr(right)
        }
        Expr::BoolOp { values, .. } => values.iter().any(contains_yield_in_expr),
        Expr::Compare {
            left, comparators, ..
        } => contains_yield_in_expr(left) || comparators.iter().any(contains_yield_in_expr),
        Expr::UnaryOp { operand, .. } => contains_yield_in_expr(operand),
        Expr::IfExp { test, body, orelse } => {
            contains_yield_in_expr(test)
                || contains_yield_in_expr(body)
                || contains_yield_in_expr(orelse)
        }
        Expr::Lambda { body, .. } => contains_yield_in_expr(body),
        Expr::Call {
            func,
            args,
            keywords,
        } => {
            contains_yield_in_expr(func)
                || args.iter().any(contains_yield_in_expr)
                || keywords.iter().any(|k| contains_yield_in_expr(&k.value))
        }
        Expr::Attribute { value, .. } => contains_yield_in_expr(value),
        Expr::Subscript { value, slice } => {
            contains_yield_in_expr(value) || contains_yield_in_expr(slice)
        }
        Expr::List(elts) | Expr::Tuple(elts) => elts.iter().any(contains_yield_in_expr),
        Expr::Dict { keys, values } => {
            keys.iter()
                .any(|k| k.as_ref().map_or(false, |e| contains_yield_in_expr(e)))
                || values.iter().any(contains_yield_in_expr)
        }
        Expr::Starred(expr) => contains_yield_in_expr(expr),
        Expr::ListComp { elt, generators } | Expr::SetComp { elt, generators } => {
            contains_yield_in_expr(elt)
                || generators.iter().any(|g| {
                    contains_yield_in_expr(&g.iter)
                        || contains_yield_in_expr(&g.target)
                        || g.ifs.iter().any(|e| contains_yield_in_expr(e))
                })
        }
        Expr::DictComp {
            key,
            value,
            generators,
        } => {
            contains_yield_in_expr(key)
                || contains_yield_in_expr(value)
                || generators.iter().any(|g| {
                    contains_yield_in_expr(&g.iter)
                        || contains_yield_in_expr(&g.target)
                        || g.ifs.iter().any(|e| contains_yield_in_expr(e))
                })
        }
        Expr::GeneratorExp { elt, generators } => {
            contains_yield_in_expr(elt)
                || generators.iter().any(|g| {
                    contains_yield_in_expr(&g.iter)
                        || contains_yield_in_expr(&g.target)
                        || g.ifs.iter().any(|e| contains_yield_in_expr(e))
                })
        }
        // An f-string's embedded expressions can contain `await` (legal in
        // an async function: `f"{await foo()}"`) — see the matching fix in
        // `collect_names_expr` for why treating the whole f-string as
        // opaque is wrong in general.
        Expr::FString(parts) => parts.iter().any(|p| match p {
            FStringPart::Expr {
                expr, format_spec, ..
            } => {
                contains_yield_in_expr(expr)
                    || format_spec
                        .as_ref()
                        .is_some_and(|fs| contains_yield_in_expr(fs))
            }
            FStringPart::String(_) => false,
        }),
        Expr::JoinedStr(exprs) => exprs.iter().any(contains_yield_in_expr),
        _ => false,
    }
}
