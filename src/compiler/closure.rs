use crate::ast::*;
use crate::bytecode::*;
use std::collections::HashSet;
use super::Compiler;

impl Compiler {
    // ---- Closure analysis ----

    pub(crate) fn collect_names_expr(expr: &Expr, names: &mut HashSet<String>) {
        match expr {
            Expr::Name(n) => {
                names.insert(n.clone());
            }
            Expr::Constant(_) => {}
            // An f-string's embedded `{expr}`/`{expr:{format_spec}}` parts
            // are real expressions that can reference any name in scope —
            // including a closure variable from an enclosing function
            // (real code: `f"{func_name}() takes at most {max_positional_args}
            // ..."` inside a nested function, `django.utils.deprecation`).
            // Treating the whole f-string as opaque (the previous `=> {}`
            // no-op) made such names invisible to this scan, so the
            // upfront cell/free-variable analysis never learned the
            // enclosing function needed to expose them as cells — the same
            // class of bug as the control-flow-recursion fix above, just
            // triggered by a different AST shape.
            Expr::FString(parts) => {
                for part in parts {
                    if let FStringPart::Expr {
                        expr, format_spec, ..
                    } = part
                    {
                        Self::collect_names_expr(expr, names);
                        if let Some(fs) = format_spec {
                            Self::collect_names_expr(fs, names);
                        }
                    }
                }
            }
            Expr::JoinedStr(exprs) => {
                for e in exprs {
                    Self::collect_names_expr(e, names);
                }
            }
            Expr::BinOp { left, right, .. } => {
                Self::collect_names_expr(left, names);
                Self::collect_names_expr(right, names);
            }
            Expr::UnaryOp { operand, .. } => Self::collect_names_expr(operand, names),
            Expr::BoolOp { values, .. } => {
                for v in values {
                    Self::collect_names_expr(v, names);
                }
            }
            Expr::Compare {
                left, comparators, ..
            } => {
                Self::collect_names_expr(left, names);
                for c in comparators {
                    Self::collect_names_expr(c, names);
                }
            }
            Expr::Call {
                func,
                args,
                keywords,
            } => {
                Self::collect_names_expr(func, names);
                for a in args {
                    Self::collect_names_expr(a, names);
                }
                for kw in keywords {
                    Self::collect_names_expr(&kw.value, names);
                }
                // A bare `super()` (PEP 3135) implicitly references the
                // textually-enclosing class via the `__class__` name — it
                // must be collected so the free-variable analysis wires the
                // `__class__` cell for methods (without this, `__class__`
                // never appears in a method's freevars and bare super in a
                // class nested in a function falls back to LOAD_GLOBAL of
                // the class name, which isn't a global there).
                if args.is_empty() && keywords.is_empty() {
                    if let Expr::Name(n) = func.as_ref() {
                        if n == "super" {
                            names.insert("__class__".to_string());
                        }
                    }
                }
            }
            Expr::IfExp { test, body, orelse } => {
                Self::collect_names_expr(test, names);
                Self::collect_names_expr(body, names);
                Self::collect_names_expr(orelse, names);
            }
            Expr::Attribute { value, .. } => Self::collect_names_expr(value, names),
            Expr::Subscript { value, slice } => {
                Self::collect_names_expr(value, names);
                Self::collect_names_expr(slice, names);
            }
            Expr::Starred(expr) => Self::collect_names_expr(expr, names),
            Expr::List(elts) | Expr::Tuple(elts) | Expr::Set(elts) => {
                for e in elts {
                    Self::collect_names_expr(e, names);
                }
            }
            Expr::Dict { keys, values } => {
                for k in keys.iter().flatten() {
                    Self::collect_names_expr(k, names);
                }
                for v in values {
                    Self::collect_names_expr(v, names);
                }
            }
            Expr::Slice { lower, upper, step } => {
                for s in [lower, upper, step].iter().filter_map(|s| s.as_ref()) {
                    Self::collect_names_expr(s, names);
                }
            }
            Expr::Lambda { body, .. } => Self::collect_names_expr(body, names),
            Expr::Yield(Some(e)) | Expr::YieldFrom(e) | Expr::Await(e) => {
                Self::collect_names_expr(e, names)
            }
            Expr::Yield(None) => {}
            // A comprehension/genexpr's `for target in ...` binds `target`
            // within its own scope — it is not a reference to anything
            // from the enclosing function, and must not be reported as
            // one. The previous code fed `gen.target` straight into the
            // same `names` set as everything else, so e.g. `any(... for
            // name in xs)` made "name" look like a free reference the
            // enclosing function needed to supply — which, once something
            // elsewhere also legitimately needed "name" relayed as a
            // closure (a `for name in ...` genexpr inside a *different*
            // nested function), caused the enclosing function's cellvars
            // list to gain an extra, unsorted, incrementally-added entry
            // for "name" *after* other cellvar-relative LOAD_DEREF indices
            // had already been emitted assuming a smaller list — silently
            // shifting them to the wrong variable (confirmed via
            // `django.utils.deprecation.deprecate_posargs`, whose `any(...
            // for name in remappable_names)` and a *separate* nested
            // function's own `for name in ...` genexpr collided exactly
            // this way). Only the first generator's `iter` is genuinely
            // evaluated in the enclosing scope (real Python semantics — it
            // becomes the genexpr's own initial argument); `elt`, every
            // other `iter`, and all `ifs` run inside the comprehension's
            // scope, where every generator's `target` is already bound, so
            // names matching one of those don't propagate outward either.
            Expr::ListComp { elt, generators }
            | Expr::SetComp { elt, generators }
            | Expr::GeneratorExp { elt, generators } => {
                let mut bound = HashSet::new();
                for gen in generators {
                    Self::collect_names_expr(&gen.target, &mut bound);
                }
                if let Some(first) = generators.first() {
                    Self::collect_names_expr(&first.iter, names);
                }
                let mut inner = HashSet::new();
                Self::collect_names_expr(elt, &mut inner);
                for (i, gen) in generators.iter().enumerate() {
                    if i > 0 {
                        Self::collect_names_expr(&gen.iter, &mut inner);
                    }
                    for if_cond in &gen.ifs {
                        Self::collect_names_expr(if_cond, &mut inner);
                    }
                }
                for n in inner {
                    if !bound.contains(&n) {
                        names.insert(n);
                    }
                }
            }
            Expr::DictComp {
                key,
                value,
                generators,
            } => {
                let mut bound = HashSet::new();
                for gen in generators {
                    Self::collect_names_expr(&gen.target, &mut bound);
                }
                if let Some(first) = generators.first() {
                    Self::collect_names_expr(&first.iter, names);
                }
                let mut inner = HashSet::new();
                Self::collect_names_expr(key, &mut inner);
                Self::collect_names_expr(value, &mut inner);
                for (i, gen) in generators.iter().enumerate() {
                    if i > 0 {
                        Self::collect_names_expr(&gen.iter, &mut inner);
                    }
                    for if_cond in &gen.ifs {
                        Self::collect_names_expr(if_cond, &mut inner);
                    }
                }
                for n in inner {
                    if !bound.contains(&n) {
                        names.insert(n);
                    }
                }
            }
            Expr::NamedExpr { target, value } => {
                Self::collect_names_expr(target, names);
                Self::collect_names_expr(value, names);
            }
        }
    }

    /// Find names assigned in a body (targets of =, for, function defs, etc.)
    pub(crate) fn collect_assigned_names(stmts: &[Stmt]) -> HashSet<String> {
        let mut assigned = HashSet::new();
        Self::collect_assigned_inner(stmts, &mut assigned);
        assigned
    }

    pub(crate) fn collect_assigned_inner(stmts: &[Stmt], assigned: &mut HashSet<String>) {
        for stmt in stmts {
            let stmt = Self::unwrap_located(stmt);
            match stmt {
                Stmt::Assign { targets, .. } => {
                    for t in targets {
                        Self::collect_assign_target_names(t, assigned);
                    }
                }
                Stmt::AugAssign { target, .. } => {
                    Self::collect_assign_target_names(target, assigned);
                }
                Stmt::AnnAssign {
                    target,
                    value: Some(_),
                    ..
                } => {
                    Self::collect_assign_target_names(target, assigned);
                }
                Stmt::For {
                    target,
                    body,
                    orelse,
                    ..
                } => {
                    Self::collect_assign_target_names(target, assigned);
                    Self::collect_assigned_inner(body, assigned);
                    Self::collect_assigned_inner(orelse, assigned);
                }
                Stmt::FunctionDef { name, .. } => {
                    assigned.insert(name.clone());
                }
                Stmt::ClassDef { name, .. } => {
                    assigned.insert(name.clone());
                }
                Stmt::If { body, orelse, .. } => {
                    Self::collect_assigned_inner(body, assigned);
                    Self::collect_assigned_inner(orelse, assigned);
                }
                Stmt::While { body, orelse, .. } => {
                    Self::collect_assigned_inner(body, assigned);
                    Self::collect_assigned_inner(orelse, assigned);
                }
                Stmt::With { items, body, .. } => {
                    for item in items {
                        if let Some(var) = &item.optional_vars {
                            Self::collect_assign_target_names(var, assigned);
                        }
                    }
                    Self::collect_assigned_inner(body, assigned);
                }
                Stmt::Match { cases, .. } => {
                    for case in cases {
                        Self::collect_assigned_inner(&case.body, assigned);
                    }
                }
                Stmt::Try {
                    body,
                    handlers,
                    handlers_star: _,
                    orelse,
                    finalbody,
                } => {
                    Self::collect_assigned_inner(body, assigned);
                    for h in handlers {
                        Self::collect_assigned_inner(&h.body, assigned);
                    }
                    Self::collect_assigned_inner(orelse, assigned);
                    Self::collect_assigned_inner(finalbody, assigned);
                }
                Stmt::Import(names_list) => {
                    for alias in names_list {
                        assigned.insert(alias.asname.clone().unwrap_or_else(|| alias.name.clone()));
                    }
                }
                Stmt::ImportFrom {
                    names: names_list, ..
                } => {
                    for alias in names_list {
                        assigned.insert(alias.asname.clone().unwrap_or_else(|| alias.name.clone()));
                    }
                }
                _ => {}
            }
        }
    }

    pub(crate) fn collect_assign_target_names(target: &Expr, assigned: &mut HashSet<String>) {
        match target {
            Expr::Name(n) => {
                assigned.insert(n.clone());
            }
            Expr::List(elts) | Expr::Tuple(elts) => {
                for e in elts {
                    Self::collect_assign_target_names(e, assigned);
                }
            }
            Expr::Starred(e) => Self::collect_assign_target_names(e, assigned),
            _ => {}
        }
    }

    /// Collect names referenced in the current function's own body (NOT nested function bodies).
    /// Names referenced anywhere in this function's own body — including
    /// inside `if`/`while`/`for`/`with`/`try`/`match` bodies, which don't
    /// introduce a new Python scope, so a name used only inside one of
    /// those (e.g. `def outer(x):\n def inner():\n  if True: return x`)
    /// must still be recognized as needing to come from an enclosing scope.
    /// Does NOT descend into nested FunctionDef/ClassDef bodies — those have
    /// their own scope and are handled separately (collect_nested_references).
    pub(crate) fn collect_own_referenced_names(stmts: &[Stmt]) -> HashSet<String> {
        let mut names = HashSet::new();
        Self::collect_own_referenced_names_inner(stmts, &mut names);
        names
    }

    pub(crate) fn collect_own_referenced_names_inner(stmts: &[Stmt], names: &mut HashSet<String>) {
        for stmt in stmts {
            let stmt = Self::unwrap_located(stmt);
            match stmt {
                Stmt::Located(..) => unreachable!("stmt already unwrapped via unwrap_located"),
                Stmt::Expr(expr) => Self::collect_names_expr(expr, names),
                Stmt::Return(Some(expr)) => Self::collect_names_expr(expr, names),
                Stmt::Return(None) | Stmt::Pass | Stmt::Break | Stmt::Continue => {}
                Stmt::Assign { targets, value } => {
                    Self::collect_names_expr(value, names);
                    for t in targets {
                        Self::collect_names_expr(t, names);
                    }
                }
                Stmt::AugAssign { target, value, .. } => {
                    Self::collect_names_expr(target, names);
                    Self::collect_names_expr(value, names);
                }
                Stmt::AnnAssign { target, value, .. } => {
                    Self::collect_names_expr(target, names);
                    if let Some(v) = value {
                        Self::collect_names_expr(v, names);
                    }
                }
                Stmt::If { test, body, orelse } => {
                    Self::collect_names_expr(test, names);
                    Self::collect_own_referenced_names_inner(body, names);
                    Self::collect_own_referenced_names_inner(orelse, names);
                }
                Stmt::While { test, body, orelse } => {
                    Self::collect_names_expr(test, names);
                    Self::collect_own_referenced_names_inner(body, names);
                    Self::collect_own_referenced_names_inner(orelse, names);
                }
                Stmt::For {
                    target,
                    iter,
                    body,
                    orelse,
                    ..
                } => {
                    Self::collect_names_expr(target, names);
                    Self::collect_names_expr(iter, names);
                    Self::collect_own_referenced_names_inner(body, names);
                    Self::collect_own_referenced_names_inner(orelse, names);
                }
                Stmt::With { items, body, .. } => {
                    for item in items {
                        Self::collect_names_expr(&item.context_expr, names);
                        if let Some(var) = &item.optional_vars {
                            Self::collect_names_expr(var, names);
                        }
                    }
                    Self::collect_own_referenced_names_inner(body, names);
                }
                Stmt::Try {
                    body,
                    handlers,
                    handlers_star,
                    orelse,
                    finalbody,
                } => {
                    Self::collect_own_referenced_names_inner(body, names);
                    for h in handlers {
                        if let Some(t) = &h.typ {
                            Self::collect_names_expr(t, names);
                        }
                        Self::collect_own_referenced_names_inner(&h.body, names);
                    }
                    for h in handlers_star {
                        if let Some(t) = &h.typ {
                            Self::collect_names_expr(t, names);
                        }
                        Self::collect_own_referenced_names_inner(&h.body, names);
                    }
                    Self::collect_own_referenced_names_inner(orelse, names);
                    Self::collect_own_referenced_names_inner(finalbody, names);
                }
                Stmt::Raise { exc, cause } => {
                    if let Some(e) = exc {
                        Self::collect_names_expr(e, names);
                    }
                    if let Some(c) = cause {
                        Self::collect_names_expr(c, names);
                    }
                }
                Stmt::Assert { test, msg } => {
                    Self::collect_names_expr(test, names);
                    if let Some(m) = msg {
                        Self::collect_names_expr(m, names);
                    }
                }
                Stmt::Match { subject, cases } => {
                    Self::collect_names_expr(subject, names);
                    for case in cases {
                        if let Some(guard) = &case.guard {
                            Self::collect_names_expr(guard, names);
                        }
                        Self::collect_own_referenced_names_inner(&case.body, names);
                    }
                }
                Stmt::Delete(targets) => {
                    for t in targets {
                        Self::collect_names_expr(t, names);
                    }
                }
                Stmt::TypeAlias { value, .. } => Self::collect_names_expr(value, names),
                Stmt::FunctionDef {
                    args,
                    decorator_list,
                    returns,
                    ..
                } => {
                    // A nested function's DEFAULT values, decorators, and
                    // return annotation are evaluated in the ENCLOSING scope
                    // (the class body or function that contains the def), so
                    // names they reference must be counted as references of
                    // THIS scope — e.g. a class-body `def f(self, x=module):`
                    // needs `module` to be a free variable of the class body.
                    for arg in args {
                        if let Some(d) = &arg.default {
                            Self::collect_names_expr(d, names);
                        }
                        if let Some(a) = &arg.annotation {
                            Self::collect_names_expr(a, names);
                        }
                    }
                    for d in decorator_list {
                        Self::collect_names_expr(d, names);
                    }
                    if let Some(r) = returns {
                        Self::collect_names_expr(r, names);
                    }
                }
                Stmt::ClassDef { .. } => {}
                Stmt::Import(_) | Stmt::ImportFrom { .. } | Stmt::Global(_) | Stmt::Nonlocal(_) => {
                }
            }
        }
    }

    /// Pre-analyze a function body to determine cell variables and free variables.
    /// Returns (cellvars, freevars)
    pub(crate) fn analyze_function(
        args: &[Arg],
        body: &[Stmt],
        global_names: &HashSet<String>,
        nonlocal_names: &HashSet<String>,
        enclosing_names: Option<&HashSet<String>>,
    ) -> (Vec<String>, Vec<String>, HashSet<String>) {
        // Find nonlocal declarations within this function's body
        let (body_globals, body_nonlocals) = Self::scan_global_nonlocal_decls(body);
        let mut effective_global = global_names.clone();
        let mut effective_nonlocal = nonlocal_names.clone();
        effective_global.extend(body_globals);
        effective_nonlocal.extend(body_nonlocals);

        // Collect all names assigned locally (including params)
        let mut local_names = Self::collect_assigned_names(body);
        for arg in args {
            local_names.insert(arg.arg.clone());
        }
        for n in &effective_nonlocal {
            local_names.remove(n);
        }
        for n in &effective_global {
            local_names.remove(n);
        }

        // Collect names referenced in THIS function's own body
        let own_refs = Self::collect_own_referenced_names(body);

        // Collect names referenced in nested function definitions
        let nested_refs = Self::collect_nested_references(
            body,
            &local_names,
            &effective_global,
            &effective_nonlocal,
        );

        // All names from outer scope = own_refs (not local) + nested_refs.
        // nested_refs may now include names needed by something nested two
        // or more levels down (relayed transitively through intervening
        // scopes) — only keep those that are either satisfiable by our own
        // locals (cellvar candidates, handled below) or genuinely available
        // from further out; anything else is a plain global/builtin and
        // must NOT be dragged in here.
        let mut all_outer_refs: HashSet<String> = nested_refs
            .iter()
            .filter(|n| {
                local_names.contains(*n) || enclosing_names.map_or(true, |en| en.contains(*n))
            })
            .cloned()
            .collect();
        for name in &own_refs {
            if !local_names.contains(name)
                && !effective_global.contains(name)
                && enclosing_names.map_or(true, |en| en.contains(name))
            {
                all_outer_refs.insert(name.clone());
            }
        }

        // cell_vars = names a nested function needs that we must expose as
        // a cell — either because it's genuinely one of our own locals
        // (the original `local_names ∩ nested_refs` case), OR because it's
        // itself a free variable we only received from *our* enclosing
        // scope but a function nested inside *us* also needs it relayed
        // through (real code: `deprecate_posargs(deprecation_warning,
        // remappable_names, /)`'s nested `decorator` receives both as free
        // variables from `deprecate_posargs`, but `decorator`'s own nested
        // `remap_deprecated_args`/genexpr also reference them — so
        // `decorator` must re-expose them as cells, not just read them as
        // plain free variables). Missing this second case previously meant
        // such a name was only ever a free variable here, with no matching
        // cell — the nested function's own free-variable *index* (computed
        // against `cellvars.len() + position`, see `Expr::Name`'s LOAD_DEREF
        // emission) then silently pointed at a different, unrelated
        // variable once the enclosing function's real cellvars list (built
        // incrementally as nested closures compile) didn't match what this
        // upfront pass had promised. `all_outer_refs` (below) already holds
        // every name available from further out that could need this
        // treatment.
        let mut cell_vars: Vec<String> = local_names
            .intersection(&nested_refs)
            .filter(|n| !effective_global.contains(*n))
            .cloned()
            .collect();
        for name in all_outer_refs.intersection(&nested_refs) {
            if !local_names.contains(name)
                && !effective_global.contains(name)
                && !cell_vars.contains(name)
            {
                cell_vars.push(name.clone());
            }
        }
        cell_vars.sort();

        // free_vars = all_outer_refs - local_names (excluding global)
        let mut free_vars: Vec<String> = all_outer_refs
            .difference(&local_names)
            .filter(|n| !effective_global.contains(*n))
            .cloned()
            .collect();
        // Also include name referenced directly in this function that aren't local
        // but only if they exist in an enclosing function's scope (not module globals)
        for name in &own_refs {
            if !local_names.contains(name)
                && !free_vars.contains(name)
                && !effective_global.contains(name)
                && enclosing_names.map_or(true, |en| en.contains(name))
            {
                free_vars.push(name.clone());
            }
        }
        // Include explicit nonlocal declarations
        for n in &effective_nonlocal {
            if !free_vars.contains(n) && !effective_global.contains(n) {
                free_vars.push(n.clone());
            }
        }
        free_vars.sort();

        (cell_vars, free_vars, local_names)
    }

    /// Recursively find names referenced in nested function bodies that are NOT
    /// assigned within those nested functions.
    pub(crate) fn collect_nested_references(
        stmts: &[Stmt],
        local_names: &HashSet<String>,
        global_names: &HashSet<String>,
        nonlocal_names: &HashSet<String>,
    ) -> HashSet<String> {
        let mut refs = HashSet::new();
        Self::collect_nested_refs_inner(
            stmts,
            local_names,
            global_names,
            nonlocal_names,
            &mut refs,
        );
        refs
    }

    pub(crate) fn collect_nested_refs_inner(
        stmts: &[Stmt],
        local_names: &HashSet<String>,
        global_names: &HashSet<String>,
        nonlocal_names: &HashSet<String>,
        refs: &mut HashSet<String>,
    ) {
        for stmt in stmts {
            let stmt = Self::unwrap_located(stmt);
            match stmt {
                Stmt::FunctionDef { args, body, .. } => {
                    let (inner_globals, inner_nonlocals) = Self::scan_global_nonlocal_decls(body);
                    let mut inner_local = Self::collect_assigned_names(body);
                    for arg in args {
                        inner_local.insert(arg.arg.clone());
                    }
                    for n in &inner_nonlocals {
                        inner_local.remove(n);
                    }
                    for n in &inner_globals {
                        inner_local.remove(n);
                    }
                    // Names this nested function references directly that
                    // aren't its own locals — it needs these from an
                    // enclosing scope (either us, or further out still).
                    let own_refs = Self::collect_own_referenced_names(body);
                    for name in &own_refs {
                        if !inner_local.contains(name) && !inner_globals.contains(name) {
                            refs.insert(name.clone());
                        }
                    }
                    // Recurse: anything referenced by a function/class
                    // nested even deeper that isn't satisfied by THIS
                    // function's own locals also needs to come from further
                    // out than this function, i.e. from us or beyond.
                    let mut deeper = HashSet::new();
                    Self::collect_nested_refs_inner(
                        body,
                        &inner_local,
                        &inner_globals,
                        &inner_nonlocals,
                        &mut deeper,
                    );
                    for name in deeper {
                        if !inner_local.contains(&name) {
                            refs.insert(name);
                        }
                    }
                }
                // Class bodies are transparent for closure purposes: a method
                // defined inside a class inside a function can still close
                // over the function's locals (Python skips class scopes when
                // resolving enclosing references), so keep looking inside
                // using the same local_names as our caller.
                Stmt::ClassDef {
                    bases,
                    keywords,
                    decorator_list,
                    body,
                    ..
                } => {
                    // `bases`/`keywords`/`decorator_list` are ordinary
                    // expressions evaluated in the ENCLOSING scope at the
                    // point of the `class` statement itself (NOT inside the
                    // class body) — e.g. `class Inner(metaclass=x):` needs
                    // `x` from whatever scope contains this statement,
                    // exactly like any other expression there. This was
                    // completely unscanned (only `body` was ever looked at),
                    // so a name referenced ONLY in a base/keyword/decorator
                    // expression — never inside the class body itself —
                    // was invisible to this upfront free-variable analysis:
                    // the enclosing function never learned it needed to
                    // expose that name as a cell, so `LOAD_DEREF` for it was
                    // never wired up, raising `NameError` at the point the
                    // class statement actually ran. Real trigger: CPython's
                    // own `test_abc.py`'s `test_factory(abc_ABCMeta, ...)`
                    // pattern — a method (itself nested in a class nested in
                    // a function) containing `class C(metaclass=abc_ABCMeta):`,
                    // where `abc_ABCMeta` is the outer function's own
                    // parameter, referenced ONLY as that keyword's value.
                    let mut header_refs = HashSet::new();
                    for base in bases {
                        Self::collect_names_expr(base, &mut header_refs);
                    }
                    for kw in keywords {
                        Self::collect_names_expr(&kw.value, &mut header_refs);
                    }
                    for dec in decorator_list {
                        Self::collect_names_expr(dec, &mut header_refs);
                    }
                    // Only names NOT already resolvable as a local/global of
                    // the scope directly containing this `class` statement
                    // need to come from further out — matching the
                    // `FunctionDef` arm's own filtering just above.
                    for name in header_refs {
                        if !local_names.contains(&name) && !global_names.contains(&name) {
                            refs.insert(name);
                        }
                    }
                    // The class BODY is its own scope, but names it references
                    // that aren't class-body locals come from the enclosing
                    // function (CPython resolves class-body names by skipping
                    // class scopes) — e.g. a method default `module=module`
                    // inside a class nested in a function needs the enclosing
                    // function to expose `module` as a cell. Previously only
                    // NESTED definitions inside the body were scanned, so such
                    // direct class-body references never reached the enclosing
                    // function's cellvar analysis -> NameError at class build.
                    let class_local = Self::collect_assigned_names(body);
                    let body_own_refs = Self::collect_own_referenced_names(body);
                    for name in body_own_refs {
                        if !class_local.contains(&name)
                            && !local_names.contains(&name)
                            && !global_names.contains(&name)
                            && !nonlocal_names.contains(&name)
                        {
                            refs.insert(name);
                        }
                    }
                    Self::collect_nested_refs_inner(
                        body,
                        local_names,
                        global_names,
                        nonlocal_names,
                        refs,
                    );
                }
                // Control-flow statements are NOT their own scope — a nested
                // `def`/`class` inside an `if`/`while`/`for`/`try`/`with`
                // body is exactly as much a "nested function of the
                // enclosing function" as one written directly at its top
                // level (real code: `if iscoroutinefunction(func): async
                // def wrapper(...): ... else: def wrapper(...): ...`, a
                // completely ordinary sync/async-dispatching decorator
                // pattern). Previously these fell to the catch-all no-op
                // below, so a closure captured *only* by a conditionally-
                // defined nested function was never added to the enclosing
                // function's `cell_vars` during this upfront static pass —
                // it only got added later, lazily, while actually compiling
                // that nested function's closure-building code (see
                // compile_function's "Emit LOAD_CLOSURE" step) — by which
                // point any *other* free-variable reference already
                // compiled earlier in the enclosing function's own body
                // (e.g. the `if` condition itself) had already emitted a
                // `LOAD_DEREF` index computed against the *old, smaller*
                // `cellvars` list, silently going stale once `cellvars`
                // grew. (Cellvars sort before freevars in the combined
                // LOAD_DEREF index space, so any resulting off-by-one loads
                // the wrong variable outright — confirmed via a minimal
                // repro where an `if <closed-over free var>:` branch always
                // took the same path regardless of the free var's real
                // value, because index 0 pointed at a *cell* var instead.)
                Stmt::If { body, orelse, .. } => {
                    Self::collect_nested_refs_inner(
                        body,
                        local_names,
                        global_names,
                        nonlocal_names,
                        refs,
                    );
                    Self::collect_nested_refs_inner(
                        orelse,
                        local_names,
                        global_names,
                        nonlocal_names,
                        refs,
                    );
                }
                Stmt::While { body, orelse, .. } => {
                    Self::collect_nested_refs_inner(
                        body,
                        local_names,
                        global_names,
                        nonlocal_names,
                        refs,
                    );
                    Self::collect_nested_refs_inner(
                        orelse,
                        local_names,
                        global_names,
                        nonlocal_names,
                        refs,
                    );
                }
                Stmt::For { body, orelse, .. } => {
                    Self::collect_nested_refs_inner(
                        body,
                        local_names,
                        global_names,
                        nonlocal_names,
                        refs,
                    );
                    Self::collect_nested_refs_inner(
                        orelse,
                        local_names,
                        global_names,
                        nonlocal_names,
                        refs,
                    );
                }
                Stmt::With { body, .. } => {
                    Self::collect_nested_refs_inner(
                        body,
                        local_names,
                        global_names,
                        nonlocal_names,
                        refs,
                    );
                }
                Stmt::Try {
                    body,
                    handlers,
                    handlers_star,
                    orelse,
                    finalbody,
                } => {
                    Self::collect_nested_refs_inner(
                        body,
                        local_names,
                        global_names,
                        nonlocal_names,
                        refs,
                    );
                    for h in handlers {
                        Self::collect_nested_refs_inner(
                            &h.body,
                            local_names,
                            global_names,
                            nonlocal_names,
                            refs,
                        );
                    }
                    for h in handlers_star {
                        Self::collect_nested_refs_inner(
                            &h.body,
                            local_names,
                            global_names,
                            nonlocal_names,
                            refs,
                        );
                    }
                    Self::collect_nested_refs_inner(
                        orelse,
                        local_names,
                        global_names,
                        nonlocal_names,
                        refs,
                    );
                    Self::collect_nested_refs_inner(
                        finalbody,
                        local_names,
                        global_names,
                        nonlocal_names,
                        refs,
                    );
                }
                _ => {}
            }
        }
    }

    pub(crate) fn scan_global_nonlocal_decls(body: &[Stmt]) -> (HashSet<String>, HashSet<String>) {
        let mut globals = HashSet::new();
        let mut nonlocals = HashSet::new();
        for stmt in body {
            let stmt = Self::unwrap_located(stmt);
            match stmt {
                Stmt::Global(names) => {
                    for n in names {
                        globals.insert(n.clone());
                    }
                }
                Stmt::Nonlocal(names) => {
                    for n in names {
                        nonlocals.insert(n.clone());
                    }
                }
                _ => {}
            }
        }
        (globals, nonlocals)
    }

}
