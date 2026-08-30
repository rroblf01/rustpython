use crate::ast::*;
use crate::token::Token;

use super::Parser;

/// True if `msg` already starts with the `L<digits>:<digits>:` position
/// prefix — mirrors exactly what `PyError::syntax_error_with_filename`'s
/// own prefix parser accepts, so this stays in sync with what it would
/// actually strip back out.
fn has_line_col_prefix(msg: &str) -> bool {
    msg.strip_prefix('L')
        .and_then(|rest| rest.split_once(':'))
        .map(|(ln, rest)| ln.parse::<i64>().is_ok() && rest.split_once(':').is_some_and(|(col, _)| col.parse::<i64>().is_ok()))
        .unwrap_or(false)
}

impl Parser {
    pub(crate) fn parse_stmt(&mut self) -> Result<Stmt, String> {
        // Lexer-detected errors (e.g. unindent does not match) surface as a
        // special current token; surface them as parser errors.
        if let Token::LexerError(msg) = &self.current {
            // Match the `L<line>:<col>: <message>` convention every other
            // syntax error here uses (see the `unexpected indent` arm just
            // below) so the resulting SyntaxError gets a real line/offset/
            // text instead of defaulting to the generic fallback position
            // — a lexer-detected error (unterminated string, line
            // continuation at EOF, ...) was losing its position entirely.
            // Some lexer errors (e.g. "invalid character ...") already
            // bake their OWN `L<line>:<col>:` prefix in at the tokenizer
            // level — double-prefixing those broke test_unicode_
            // identifiers.py (the outer prefix's line/col got stripped by
            // the eventual SyntaxError constructor, but the INNER one
            // stayed embedded as literal text in the displayed message).
            // Only add it when it's not already there.
            if has_line_col_prefix(msg) {
                return Err(msg.clone());
            }
            let (line, col) = self.lexer.get_line_col();
            return Err(format!("L{}:{}: {}", line, col, msg));
        }
        // An Indent token here means indentation that doesn't belong to any
        // compound-statement suite — i.e. an unexpected indent.
        if self.at(&Token::Indent) {
            let (line, col) = self.lexer.get_line_col();
            return Err(format!("L{}:{}: unexpected indent", line, col));
        }
        self.parse_simple_stmt()
    }

    pub(crate) fn parse_simple_stmt(&mut self) -> Result<Stmt, String> {
        if self.at(&Token::Pass) {
            self.next();
            let _ = self.expect_newline_or_eof();
            return Ok(Stmt::Pass);
        }
        if self.at(&Token::Break) {
            self.next();
            let _ = self.expect_newline_or_eof();
            return Ok(Stmt::Break);
        }
        if self.at(&Token::Continue) {
            self.next();
            let _ = self.expect_newline_or_eof();
            return Ok(Stmt::Continue);
        }
        if self.at(&Token::Return) {
            return self.parse_return();
        }
        if self.at(&Token::Yield) {
            return self.parse_yield_stmt();
        }
        if self.at(&Token::Raise) {
            return self.parse_raise();
        }
        if self.at(&Token::Global) {
            return self.parse_global();
        }
        if self.at(&Token::Nonlocal) {
            return self.parse_nonlocal();
        }
        if self.at(&Token::Assert) {
            return self.parse_assert();
        }
        if self.at(&Token::Del) {
            return self.parse_del();
        }
        if self.at(&Token::Import) {
            return self.parse_import();
        }
        if self.at(&Token::From) {
            return self.parse_from_import();
        }
        if self.at(&Token::If) {
            return self.parse_if();
        }
        if self.at(&Token::While) {
            return self.parse_while();
        }
        if self.at(&Token::For) {
            return self.parse_for(false);
        }
        if self.at(&Token::With) {
            return self.parse_with(false);
        }
        if self.at(&Token::Async) && self.peek() == &Token::For {
            if self.async_depth == 0 && !self.allow_top_level_await {
                return Err("'async for' outside async function".to_string());
            }
            self.next(); // consume async
            return self.parse_for(true);
        }
        if self.at(&Token::Async) && self.peek() == &Token::With {
            if self.async_depth == 0 && !self.allow_top_level_await {
                return Err("'async with' outside async function".to_string());
            }
            self.next(); // consume async
            return self.parse_with(true);
        }
        if self.at(&Token::Try) {
            return self.parse_try();
        }
        if matches!(&self.current, Token::Name(n) if n == "match") && self.looks_like_match_stmt() {
            return self.parse_match();
        }
        // `type` soft keyword: a real type-alias statement (`type Alias =
        // int`, `type Alias[T] = list[T]`) always has a NAME immediately
        // after `type` — nothing else starts that way, so a single-token
        // lookahead disambiguates it from `type` used as a plain identifier
        // (`type: str = ...` — an annotated assignment to a variable named
        // "type", real code in CPython's own `_colorize.py`; `type(x)`;
        // `type = 5`; `type.attr`). Previously unconditional, so EVERY use
        // of "type" as an ordinary name at statement-start misparsed.
        if matches!(&self.current, Token::Name(n) if n == "type")
            && matches!(self.peek(), Token::Name(_))
        {
            return self.parse_type_alias();
        }
        if self.at(&Token::Class) {
            return self.parse_class();
        }
        if self.at(&Token::At) {
            return self.parse_decorated();
        }
        if self.at(&Token::Def) || self.at(&Token::Async) && self.peek() == &Token::Def {
            return self.parse_function_def();
        }

        // Parse assignment target(s) which may include star unpacking
        let first = if self.at(&Token::Star) {
            self.next(); // consume *
            Expr::Starred(Box::new(self.parse_expr()?))
        } else {
            self.parse_expr()?
        };

        // Check for tuple target with comma-separated items or starred unpacking
        if self.at(&Token::Comma) {
            let mut elts = vec![first];
            loop {
                if !self.eat(&Token::Comma) {
                    break;
                }
                if self.at(&Token::Newline)
                    || self.at(&Token::Semicolon)
                    || self.at(&Token::EndOfFile)
                {
                    break;
                }
                if self.at(&Token::Star) {
                    self.next(); // consume *
                    elts.push(Expr::Starred(Box::new(self.parse_expr()?)));
                } else if self.at(&Token::Equal) {
                    // Bare comma before = means trailing comma on single-element tuple
                    break;
                } else {
                    let e = self.parse_expr()?;
                    // Expression followed by : means it's a slice: 3:4
                    if self.eat(&Token::Colon) {
                        let u = if !self.at(&Token::RightBracket) && !self.at(&Token::Comma) {
                            Some(Box::new(self.parse_expr()?))
                        } else {
                            None
                        };
                        let s = if self.eat(&Token::Colon) {
                            if !self.at(&Token::RightBracket) && !self.at(&Token::Comma) {
                                Some(Box::new(self.parse_expr()?))
                            } else {
                                None
                            }
                        } else {
                            None
                        };
                        elts.push(Expr::Slice {
                            lower: Some(Box::new(e)),
                            upper: u,
                            step: s,
                        });
                    } else {
                        elts.push(e);
                    }
                }
            }
            if self.at(&Token::Equal)
                || self.at(&Token::PlusEqual)
                || self.at(&Token::MinusEqual)
                || self.at(&Token::StarEqual)
                || self.at(&Token::SlashEqual)
                || self.at(&Token::DoubleStarEqual)
                || self.at(&Token::DoubleSlashEqual)
                || self.at(&Token::PercentEqual)
                || self.at(&Token::PipeEqual)
                || self.at(&Token::AmpersandEqual)
                || self.at(&Token::CaretEqual)
                || self.at(&Token::LeftShiftEqual)
                || self.at(&Token::RightShiftEqual)
                || self.at(&Token::AtEqual)
            {
                let tuple_expr = Expr::Tuple(elts);
                return self.parse_stmt_tail(tuple_expr);
            }
            // Not followed by assignment — treat as expression statement.
            // Reconstruct expression from the tuple elements.
            let tuple_expr = Expr::Tuple(elts);
            return self.parse_stmt_tail(tuple_expr);
        } else {
            self.parse_stmt_tail(first)
        }
    }

    pub(crate) fn parse_stmt_tail(&mut self, expr: Expr) -> Result<Stmt, String> {
        if self.eat(&Token::Equal) {
            let mut targets = vec![expr];
            // RHS may start with * for tuple unpacking: `a = *b, *c`
            let mut value = if self.at(&Token::Star) {
                let mut elts = Vec::new();
                loop {
                    if self.at(&Token::Star) {
                        self.next();
                        elts.push(Expr::Starred(Box::new(self.parse_expr()?)));
                    } else {
                        elts.push(self.parse_expr()?);
                    }
                    if !self.eat(&Token::Comma) {
                        break;
                    }
                }
                Expr::Tuple(elts)
            } else {
                let mut value = self.parse_conditional_expr()?;
                if self.at(&Token::Comma)
                    && !self.at(&Token::Semicolon)
                    && !self.at(&Token::Newline)
                {
                    let mut elts = vec![value];
                    while self.eat(&Token::Comma) {
                        if self.at(&Token::Newline)
                            || self.at(&Token::Semicolon)
                            || self.at(&Token::EndOfFile)
                        {
                            break;
                        }
                        elts.push(self.parse_conditional_expr()?);
                    }
                    value = Expr::Tuple(elts);
                }
                value
            };
            while self.eat(&Token::Equal) {
                targets.push(value);
                value = self.parse_conditional_expr()?;
                if self.at(&Token::Comma)
                    && !self.at(&Token::Semicolon)
                    && !self.at(&Token::Newline)
                {
                    let mut elts = vec![value];
                    while self.eat(&Token::Comma) {
                        if self.at(&Token::Newline)
                            || self.at(&Token::Semicolon)
                            || self.at(&Token::EndOfFile)
                        {
                            break;
                        }
                        elts.push(self.parse_conditional_expr()?);
                    }
                    value = Expr::Tuple(elts);
                }
            }
            let _ = self.expect_newline_or_eof();
            Ok(Stmt::Assign {
                targets,
                value: Box::new(value),
            })
        } else if self.at(&Token::PlusEqual)
            || self.at(&Token::MinusEqual)
            || self.at(&Token::StarEqual)
            || self.at(&Token::SlashEqual)
            || self.at(&Token::DoubleStarEqual)
            || self.at(&Token::DoubleSlashEqual)
            || self.at(&Token::PercentEqual)
            || self.at(&Token::PipeEqual)
            || self.at(&Token::AmpersandEqual)
            || self.at(&Token::CaretEqual)
            || self.at(&Token::LeftShiftEqual)
            || self.at(&Token::RightShiftEqual)
            || self.at(&Token::AtEqual)
        {
            let op = match self.next() {
                Token::PlusEqual => Operator::Add,
                Token::MinusEqual => Operator::Sub,
                Token::StarEqual => Operator::Mult,
                Token::SlashEqual => Operator::Div,
                Token::DoubleStarEqual => Operator::Pow,
                Token::DoubleSlashEqual => Operator::FloorDiv,
                Token::PercentEqual => Operator::Mod,
                Token::PipeEqual => Operator::BitOr,
                Token::AmpersandEqual => Operator::BitAnd,
                Token::CaretEqual => Operator::BitXor,
                Token::LeftShiftEqual => Operator::LShift,
                Token::RightShiftEqual => Operator::RShift,
                Token::AtEqual => Operator::MatMult,
                _ => unreachable!(),
            };
            let mut value = self.parse_conditional_expr()?;
            // Augmented assignment with tuple RHS: fds += r, w
            if self.at(&Token::Comma) {
                let mut elts = vec![value];
                while self.eat(&Token::Comma) {
                    if self.at(&Token::Newline)
                        || self.at(&Token::Semicolon)
                        || self.at(&Token::EndOfFile)
                    {
                        break;
                    }
                    elts.push(self.parse_conditional_expr()?);
                }
                value = Expr::Tuple(elts);
            }
            let _ = self.expect_newline_or_eof();
            Ok(Stmt::AugAssign {
                target: Box::new(expr),
                op,
                value: Box::new(value),
            })
        } else if self.at(&Token::Colon) {
            // Annotation assignment: x: int = 5 or x: int
            self.next(); // consume colon
            let annotation = self.parse_expr()?;
            let value = if self.eat(&Token::Equal) {
                Some(Box::new(self.parse_expr()?))
            } else {
                None
            };
            let _ = self.expect_newline_or_eof();
            Ok(Stmt::AnnAssign {
                target: Box::new(expr),
                annotation: Box::new(annotation),
                value,
            })
        } else {
            // A bare `print` followed by more tokens (`print "Hello"` — the
            // Python 2 print statement) gets CPython's dedicated hint.
            if let Expr::Name(n) = &expr {
                if n == "print"
                    && !self.at(&Token::Newline)
                    && !self.at(&Token::Semicolon)
                    && !self.at(&Token::EndOfFile)
                {
                    return Err(
                        "Missing parentheses in call to 'print'. Did you mean print(...)"
                            .to_string(),
                    );
                }
            }
            // NOTE: a generic "trailing non-terminator token" rejection here
            // would also catch valid-but-unsupported constructs the lexer
            // leaves partially consumed (rt-strings, variation-selector
            // identifiers in test_unicode_identifiers) — keep it scoped to
            // the print hint above.
            self.expect_newline_or_eof()?;
            Ok(Stmt::Expr(Box::new(expr)))
        }
    }

    pub(crate) fn expect_newline_or_eof(&mut self) -> Result<(), String> {
        // Inside a single-line compound-statement suite (`if x: a; b`),
        // do NOT consume a trailing `;` (or the closing `Newline`) here —
        // leave it in place for `parse_block`'s own explicit semicolon-loop
        // to see and act on, so it can tell whether another statement
        // follows on this same line. Every simple-statement parse path
        // calls this function to mark "I'm done" — if it ate the `;`
        // itself (as it unconditionally used to), `parse_block`'s loop
        // would never see one to eat, and any statement after the first
        // silently fell out of the suite entirely, parsed instead as a
        // separate statement in the ENCLOSING scope (see `parse_block`'s
        // own doc comment for the full, confirmed-via-repro story).
        if self.suite_depth > 0 {
            return Ok(());
        }
        if self.eat(&Token::Newline) {
            return Ok(());
        }
        if self.eat(&Token::Semicolon) {
            while self.eat(&Token::Newline) {}
            return Ok(());
        }
        if self.at(&Token::EndOfFile) {
            return Ok(());
        }
        // A trailing non-terminator token means two statements were jammed
        // together with no separator (`print "Hello World"` / `x = 1 y`) —
        // real Python raises SyntaxError (test_codeop::test_invalid asserts
        // `compile_command('a b')` fails).
        Err(format!(
            "invalid syntax: unexpected token after statement: {:?}",
            self.peek()
        ))
    }
    pub(crate) fn parse_return(&mut self) -> Result<Stmt, String> {
        self.expect(&Token::Return)?;
        let value = if !self.at(&Token::Newline)
            && !self.at(&Token::Semicolon)
            && !self.at(&Token::EndOfFile)
        {
            let first = if self.at(&Token::Star) {
                self.next();
                Expr::Starred(Box::new(self.parse_conditional_expr()?))
            } else {
                self.parse_conditional_expr()?
            };
            // return x, y → return (x, y) (tuple return)
            if self.at(&Token::Comma) {
                let mut elts = vec![first];
                while self.eat(&Token::Comma) {
                    if self.at(&Token::Newline)
                        || self.at(&Token::Semicolon)
                        || self.at(&Token::EndOfFile)
                    {
                        break;
                    }
                    if self.at(&Token::Star) {
                        self.next();
                        elts.push(Expr::Starred(Box::new(self.parse_conditional_expr()?)));
                    } else {
                        elts.push(self.parse_conditional_expr()?);
                    }
                }
                Some(Box::new(Expr::Tuple(elts)))
            } else {
                Some(Box::new(first))
            }
        } else {
            None
        };
        let _ = self.expect_newline_or_eof();
        Ok(Stmt::Return(value))
    }

    pub(crate) fn parse_yield_stmt(&mut self) -> Result<Stmt, String> {
        let expr = self.parse_yield_expr()?;
        let _ = self.expect_newline_or_eof();
        Ok(Stmt::Expr(Box::new(expr)))
    }

    pub(crate) fn parse_raise(&mut self) -> Result<Stmt, String> {
        self.next();
        let exc = if !self.at(&Token::Newline) && !self.at(&Token::EndOfFile) {
            let e = self.parse_expr()?;
            if self.eat(&Token::From) {
                let cause = self.parse_expr()?;
                let _ = self.expect_newline_or_eof();
                return Ok(Stmt::Raise {
                    exc: Some(Box::new(e)),
                    cause: Some(Box::new(cause)),
                });
            }
            Some(Box::new(e))
        } else {
            None
        };
        let _ = self.expect_newline_or_eof();
        Ok(Stmt::Raise { exc, cause: None })
    }

    pub(crate) fn parse_global(&mut self) -> Result<Stmt, String> {
        let (gline, gcol) = self.lexer.get_line_col();
        self.next();
        let mut names = vec![self.expect_name()?];
        while self.eat(&Token::Comma) {
            names.push(self.expect_name()?);
        }
        // A parameter may not be declared global (test_syntax:
        // test_global_param_err_first).
        if let Some(params) = self.fn_params_stack.last() {
            for n in &names {
                if params.contains(n) {
                    return Err(format!(
                        "L{}:{}: name '{}' is parameter and global",
                        gline, gcol, n
                    ));
                }
            }
        }
        let _ = self.expect_newline_or_eof();
        Ok(Stmt::Global(names))
    }

    pub(crate) fn parse_nonlocal(&mut self) -> Result<Stmt, String> {
        let (nline, ncol) = self.lexer.get_line_col();
        self.next();
        let mut names = vec![self.expect_name()?];
        while self.eat(&Token::Comma) {
            names.push(self.expect_name()?);
        }
        // A parameter may not be declared nonlocal (test_syntax:
        // test_nonlocal_param_err_first).
        if let Some(params) = self.fn_params_stack.last() {
            for n in &names {
                if params.contains(n) {
                    return Err(format!(
                        "L{}:{}: name '{}' is parameter and nonlocal",
                        nline, ncol, n
                    ));
                }
            }
        }
        let _ = self.expect_newline_or_eof();
        Ok(Stmt::Nonlocal(names))
    }

    pub(crate) fn parse_assert(&mut self) -> Result<Stmt, String> {
        self.next();
        let test = self.parse_expr()?;
        let msg = if self.eat(&Token::Comma) {
            Some(Box::new(self.parse_expr()?))
        } else {
            None
        };
        let _ = self.expect_newline_or_eof();
        Ok(Stmt::Assert {
            test: Box::new(test),
            msg,
        })
    }

    pub(crate) fn parse_del(&mut self) -> Result<Stmt, String> {
        self.next();
        let first = if self.at(&Token::Star) {
            self.next();
            Expr::Starred(Box::new(self.parse_expr()?))
        } else {
            self.parse_expr()?
        };
        let mut targets = vec![first];
        while self.eat(&Token::Comma) {
            if self.at(&Token::Newline) || self.at(&Token::Semicolon) || self.at(&Token::EndOfFile)
            {
                break;
            }
            let t = if self.at(&Token::Star) {
                self.next();
                Expr::Starred(Box::new(self.parse_expr()?))
            } else {
                self.parse_expr()?
            };
            targets.push(t);
        }
        self.expect_newline_or_eof()?;
        Ok(Stmt::Delete(targets))
    }

    pub(crate) fn parse_args(&mut self) -> Result<Vec<Arg>, String> {
        let mut args = Vec::new();
        // Set once a `*args` or bare `*,` separator is seen — every regular
        // param after that point is keyword-only. A bare `*,` introduces no
        // Arg of its own, so this flag is the only record that it happened;
        // without it, `def f(a, *, b, c):`'s b/c were indistinguishable from
        // plain positional params anywhere later in the compiler.
        let mut seen_star = false;
        let mut seen_slash = false;
        let mut bare_star = false;
        let mut seen_names: std::collections::HashSet<String> = std::collections::HashSet::new();
        if !self.at(&Token::RightParen) {
            loop {
                // Allow trailing comma: if we see ')' after a comma, stop
                if self.at(&Token::RightParen) {
                    break;
                }

                if self.eat(&Token::DoubleStar) {
                    // `**kw` after a BARE `*,` is invalid (real CPython:
                    // "named arguments must follow bare *") — but `**kw`
                    // after `*args` is fine (`def f(*args, **kw)`).
                    if bare_star {
                        return Err("named arguments must follow bare *".to_string());
                    }
                    let name = self.expect_name()?;
                    let annotation = if self.eat(&Token::Colon) {
                        Some(Box::new(self.parse_expr()?))
                    } else {
                        None
                    };
                    if !seen_names.insert(name.clone()) {
                        return Err(format!(
                            "duplicate argument '{}' in function definition",
                            name
                        ));
                    }
                    args.push(Arg {
                        arg: name,
                        annotation,
                        is_vararg: false,
                        is_kwarg: true,
                        is_posonlyarg: false,
                        is_kwonly: false,
                        default: None,
                    });
                    if !self.eat(&Token::Comma) {
                        break;
                    }
                } else if self.eat(&Token::Star) {
                    if self.at(&Token::RightParen) || self.at(&Token::Comma) {
                        // bare * means keyword-only args follow — but a bare
                        // `*` with NOTHING after it is an error (real
                        // CPython: "named arguments must follow bare *").
                        if self.at(&Token::RightParen) {
                            return Err("named arguments must follow bare *".to_string());
                        }
                        seen_star = true;
                        bare_star = true;
                        self.eat(&Token::Comma); // consume trailing comma if present
                        continue;
                    }
                    let name = self.expect_name()?;
                    let annotation = if self.eat(&Token::Colon) {
                        if self.eat(&Token::Star) {
                            let inner = self.parse_expr()?;
                            Some(Box::new(Expr::Starred(Box::new(inner))))
                        } else {
                            Some(Box::new(self.parse_expr()?))
                        }
                    } else {
                        None
                    };
                    if !seen_names.insert(name.clone()) {
                        return Err(format!(
                            "duplicate argument '{}' in function definition",
                            name
                        ));
                    }
                    args.push(Arg {
                        arg: name,
                        annotation,
                        is_vararg: true,
                        is_kwarg: false,
                        is_posonlyarg: false,
                        is_kwonly: false,
                        default: None,
                    });
                    seen_star = true;
                    if !self.eat(&Token::Comma) {
                        break;
                    }
                } else if self.eat(&Token::Slash) {
                    // Positional-only parameter separator '/' — marks end of positional-only params.
                    // All args parsed before this are already marked as positional-only.
                    // After '/', there's usually a comma, and then the next args are regular params.
                    // Real Python validation: '/' must follow at least one param, and
                    // must NOT follow `*`/`**` (or a bare `*,`).
                    if args.is_empty()
                        || seen_star
                        || seen_slash
                        || args.iter().any(|a| a.is_vararg || a.is_kwarg)
                    {
                        return Err("unexpected '/' in function definition".to_string());
                    }
                    seen_slash = true;
                    // Mark all existing args (that are not *vararg or **kwarg) as positional-only
                    for arg in args.iter_mut() {
                        if !arg.is_vararg && !arg.is_kwarg {
                            arg.is_posonlyarg = true;
                        }
                    }
                    if self.at(&Token::Comma) {
                        self.next();
                    }
                    // Continue parsing remaining params
                    if self.at(&Token::RightParen) {
                        break;
                    }
                    continue;
                } else {
                    let mut arg = self.parse_arg()?;
                    arg.is_kwonly = seen_star;
                    if !seen_names.insert(arg.arg.clone()) {
                        return Err(format!(
                            "duplicate argument '{}' in function definition",
                            arg.arg
                        ));
                    }
                    // A named kwonly arg satisfies the bare `*,` requirement
                    // (`def f(*, k1, **kw)` is valid; only a `**` with NO
                    // named arg since the bare `*,` is not).
                    bare_star = false;
                    args.push(arg);
                    if !self.eat(&Token::Comma) {
                        break;
                    }
                }
            }
        }
        // General rule: a positional parameter without a default cannot
        // follow one WITH a default (across the `/` boundary too).
        let mut seen_default = false;
        for arg in args.iter() {
            if arg.is_vararg || arg.is_kwarg || arg.is_kwonly {
                continue;
            }
            if arg.default.is_some() {
                seen_default = true;
            } else if seen_default {
                return Err(
                    "parameter without a default follows parameter with a default".to_string(),
                );
            }
        }
        Ok(args)
    }

    pub(crate) fn parse_arg(&mut self) -> Result<Arg, String> {
        let arg = self.expect_name()?;
        let annotation = if self.eat(&Token::Colon) {
            Some(Box::new(self.parse_expr()?))
        } else {
            None
        };
        let default = if self.eat(&Token::Equal) {
            Some(Box::new(self.parse_expr()?))
        } else {
            None
        };
        Ok(Arg {
            arg,
            annotation,
            is_vararg: false,
            is_kwarg: false,
            is_posonlyarg: false,
            is_kwonly: false,
            default,
        })
    }

    pub(crate) fn parse_block(&mut self) -> Result<Vec<Stmt>, String> {
        let mut stmts = Vec::new();
        if !self.eat(&Token::Newline) {
            // Single-line body (same line after colon) — real Python grammar
            // allows a `;`-separated CHAIN of simple statements here
            // (`simple_stmts: simple_stmt (';' simple_stmt)* [';'] NEWLINE`),
            // not just one. This previously parsed ONLY the first statement
            // and returned immediately, silently leaving any `; stmt2; ...`
            // remainder unconsumed — the enclosing statement-sequence loop
            // (`parse_program`'s own, already-correct, semicolon-handling
            // loop) would then pick those up as SEPARATE, subsequent
            // statements in the ENCLOSING scope, executed unconditionally
            // and only once (at definition/parse time), not each time the
            // compound statement's actual body should run. Confirmed via
            // multiple angles: `if False: a = 1; b = 2` left `b` set to `2`
            // despite the condition being false (only `a = 1` was really
            // gated by the `if`); `def f(x): print("A"); print("B")`
            // executed `print("B")` immediately at DEF time (not on each
            // call), before `print("A")` (which only ran when `f()` was
            // actually called) — an observable REORDERING, not just a
            // scoping leak, for any code after the def. A silent, general
            // correctness bug for any one-line `if`/`while`/`for`/`def`/
            // `class`/`with`/`try` body containing more than one semicolon-
            // separated statement — a common, unremarkable style choice in
            // real code and test suites alike, not a contrived edge case.
            self.suite_depth += 1;
            let result: Result<(), String> = (|| {
                if !self.at(&Token::Dedent) && !self.at(&Token::EndOfFile) {
                    let line = self.lexer.get_line_col().0;
                    stmts.push(Stmt::Located(line, Box::new(self.parse_stmt()?)));
                    while self.eat(&Token::Semicolon) {
                        if self.at(&Token::Newline)
                            || self.at(&Token::Dedent)
                            || self.at(&Token::EndOfFile)
                        {
                            break;
                        }
                        let line = self.lexer.get_line_col().0;
                        stmts.push(Stmt::Located(line, Box::new(self.parse_stmt()?)));
                    }
                }
                Ok(())
            })();
            self.suite_depth -= 1;
            result?;
            // The suite ends at this line's `Newline` (or EOF/Dedent) —
            // `expect_newline_or_eof` deliberately left it un-consumed
            // while `suite_depth > 0` (see its own doc comment), so eat it
            // here now that the suite itself is done. Every caller of
            // `parse_block()` already tolerates/skips a leftover `Newline`
            // afterward regardless, but eating it here keeps this
            // function's own contract (consume exactly the suite, nothing
            // more) intact rather than relying on that tolerance.
            let _ = self.eat(&Token::Newline);
            return self.finish_block(stmts);
        }
        if self.eat(&Token::Indent) {
            loop {
                match &self.current {
                    Token::Dedent => {
                        self.next();
                        return self.finish_block(stmts);
                    }
                    Token::EndOfFile => {
                        return self.finish_block(stmts);
                    }
                    _ => {}
                }
                // Skip blank lines and comment-only lines (tokenized as Newline)
                while self.at(&Token::Newline) {
                    self.next();
                }
                if self.at(&Token::Dedent) || self.at(&Token::EndOfFile) {
                    continue;
                }
                let line = self.lexer.get_line_col().0;
                stmts.push(Stmt::Located(line, Box::new(self.parse_stmt()?)));
            }
        }
        // No Indent yet — the block may begin after one or more blank
        // lines (`def x():\n\n pass\n` emits NEWLINE NEWLINE INDENT ...).
        while self.at(&Token::Newline) {
            self.next();
        }
        if self.eat(&Token::Indent) {
            return self.parse_block_indented(stmts);
        }
        self.finish_block(stmts)
    }

    pub(crate) fn parse_block_indented(&mut self, mut stmts: Vec<Stmt>) -> Result<Vec<Stmt>, String> {
        loop {
            match &self.current {
                Token::Dedent => {
                    self.next();
                    return self.finish_block(stmts);
                }
                Token::EndOfFile => {
                    return self.finish_block(stmts);
                }
                _ => {}
            }
            while self.at(&Token::Newline) {
                self.next();
            }
            if self.at(&Token::Dedent) || self.at(&Token::EndOfFile) {
                continue;
            }
            let line = self.lexer.get_line_col().0;
            stmts.push(Stmt::Located(line, Box::new(self.parse_stmt()?)));
        }
    }

    pub(crate) fn finish_block(&self, stmts: Vec<Stmt>) -> Result<Vec<Stmt>, String> {
        // `def x():\n\npass\n` / `if 1:` with no indented statements —
        // CPython raises "expected an indented block ..." (an
        // IndentationError, itself a SyntaxError). test_codeop::test_invalid
        // asserts these inputs are rejected.
        if stmts.is_empty() {
            return Err("expected an indented block".to_string());
        }
        Ok(stmts)
    }

    // ---- Expressions ----


}
