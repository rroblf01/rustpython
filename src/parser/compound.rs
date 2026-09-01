use crate::ast::*;
use crate::token::Token;

use super::Parser;

impl Parser {
    // ---- Compound statements ----

    pub(crate) fn parse_function_def(&mut self) -> Result<Stmt, String> {
        let decorator_list = Vec::new();
        let async_token = self.at(&Token::Async);
        if async_token {
            self.next(); // async
        }
        self.expect(&Token::Def)?;
        let name = self.expect_name()?;
        let type_params = if self.eat(&Token::LeftBracket) {
            let mut params = Vec::new();
            loop {
                if self.eat(&Token::Star) {
                    params.push(format!("*{}", self.expect_name()?));
                } else if self.eat(&Token::DoubleStar) {
                    params.push(format!("**{}", self.expect_name()?));
                } else {
                    params.push(self.expect_name()?);
                }
                // Optional bound: [T: str]  or [T:  str]
                if self.eat(&Token::Colon) {
                    self.parse_expr()?; // skip bound expression
                }
                // Optional default: [T = int]  or [T: str = int]
                if self.eat(&Token::Equal) {
                    self.parse_expr()?; // skip default expression
                }
                if !self.eat(&Token::Comma) {
                    break;
                }
            }
            self.expect(&Token::RightBracket)?;
            params
        } else {
            Vec::new()
        };
        self.expect(&Token::LeftParen)?;
        let args = self.parse_args()?;
        self.expect(&Token::RightParen)?;
        let returns = if self.eat(&Token::Arrow) {
            Some(Box::new(self.parse_expr()?))
        } else {
            None
        };
        // Record this function's parameter names so that `global`/`nonlocal`
        // inside the body can be validated against them (a parameter may not
        // also be declared global/nonlocal).
        let param_names: std::collections::HashSet<String> =
            args.iter().map(|a| a.arg.clone()).collect();
        self.fn_params_stack.push(param_names);
        self.expect(&Token::Colon)?;
        // `async for`/`async with` (and `await`) are only legal inside an
        // `async def` body — CPython raises SyntaxError otherwise
        // (test_builtin::test_compile's async cases).
        self.async_depth += usize::from(async_token);
        let body = self.parse_block()?;
        self.async_depth -= usize::from(async_token);
        self.fn_params_stack.pop();
        Ok(Stmt::FunctionDef {
            name,
            args,
            body,
            decorator_list,
            returns,
            is_async: async_token,
            type_params,
        })
    }

    pub(crate) fn parse_decorated(&mut self) -> Result<Stmt, String> {
        self.next();
        let mut decorator_list = vec![self.parse_expr()?];
        while self.at(&Token::At) {
            self.next();
            decorator_list.push(self.parse_expr()?);
        }
        while self.at(&Token::Newline) || self.at(&Token::Indent) || self.at(&Token::Dedent) {
            self.next();
        }
        let mut stmt = self.parse_stmt()?;
        match &mut stmt {
            Stmt::FunctionDef {
                decorator_list: d, ..
            }
            | Stmt::ClassDef {
                decorator_list: d, ..
            } => {
                // `self.parse_stmt()` above, on seeing another leading `@`,
                // recurses back into `parse_decorated` for the rest of the
                // stack — so `d` may already hold decorators collected by
                // that inner call (written closer to the `def`/`class`).
                // This level's `decorator_list` was written *before* those
                // (further from the def), so it must come first to keep
                // the final list in top-to-bottom source order — replacing
                // `d` outright (the previous behavior) silently discarded
                // every decorator but this outermost one whenever two or
                // more were stacked.
                decorator_list.extend(std::mem::take(d));
                *d = decorator_list;
            }
            _ => return Err("Decorator on non-function/class".to_string()),
        }
        Ok(stmt)
    }

    pub(crate) fn parse_class(&mut self) -> Result<Stmt, String> {
        self.expect(&Token::Class)?;
        let name = self.expect_name()?;
        let type_params = if self.eat(&Token::LeftBracket) {
            let mut params = Vec::new();
            loop {
                if self.eat(&Token::Star) {
                    params.push(format!("*{}", self.expect_name()?));
                } else if self.eat(&Token::DoubleStar) {
                    params.push(format!("**{}", self.expect_name()?));
                } else {
                    params.push(self.expect_name()?);
                }
                if self.eat(&Token::Colon) {
                    self.parse_expr()?;
                }
                if self.eat(&Token::Equal) {
                    self.parse_expr()?;
                }
                if !self.eat(&Token::Comma) {
                    break;
                }
            }
            self.expect(&Token::RightBracket)?;
            params
        } else {
            Vec::new()
        };
        let mut bases = Vec::new();
        let mut keywords = Vec::new();
        if self.eat(&Token::LeftParen) {
            if !self.at(&Token::RightParen) {
                loop {
                    if matches!(&self.current, Token::Name(_)) && self.peek() == &Token::Equal {
                        let arg = Some(self.expect_name()?);
                        self.expect(&Token::Equal)?;
                        let value = self.parse_expr()?;
                        keywords.push(Keyword {
                            arg,
                            value: Box::new(value),
                        });
                    } else if self.at(&Token::Underscore) && self.peek() == &Token::Equal {
                        self.next();
                        self.expect(&Token::Equal)?;
                        let value = self.parse_expr()?;
                        keywords.push(Keyword {
                            arg: Some("_".to_string()),
                            value: Box::new(value),
                        });
                    } else if self.eat(&Token::DoubleStar) {
                        let value = self.parse_expr()?;
                        keywords.push(Keyword {
                            arg: None,
                            value: Box::new(value),
                        });
                    } else if self.eat(&Token::Star) {
                        let value = self.parse_expr()?;
                        bases.push(Expr::Starred(Box::new(value)));
                    } else {
                        bases.push(self.parse_expr()?);
                    }
                    if !self.eat(&Token::Comma) {
                        break;
                    }
                    if self.at(&Token::RightParen) {
                        break;
                    }
                }
            }
            self.expect(&Token::RightParen)?;
        }
        self.expect(&Token::Colon)?;
        let body = self.parse_block()?;
        Ok(Stmt::ClassDef {
            name,
            bases,
            keywords,
            body,
            decorator_list: vec![],
            type_params,
        })
    }

    pub(crate) fn parse_if(&mut self) -> Result<Stmt, String> {
        self.expect(&Token::If)?;
        let test = self.parse_expr()?;
        self.expect(&Token::Colon)?;
        let body = self.parse_block()?;
        // Skip newlines/comments between if-body and elif/else
        while self.at(&Token::Newline) {
            self.next();
        }
        let mut orelse = Vec::new();
        if self.eat(&Token::Elif) {
            let elif = self.parse_if_elif()?;
            orelse.push(elif);
        } else if self.eat(&Token::Else) {
            self.expect(&Token::Colon)?;
            orelse = self.parse_block()?;
        }
        Ok(Stmt::If {
            test: Box::new(test),
            body,
            orelse,
        })
    }

    pub(crate) fn parse_if_elif(&mut self) -> Result<Stmt, String> {
        let test = self.parse_expr()?;
        self.expect(&Token::Colon)?;
        let body = self.parse_block()?;
        let mut orelse = Vec::new();
        if self.eat(&Token::Elif) {
            let elif = self.parse_if_elif()?;
            orelse.push(elif);
        } else if self.eat(&Token::Else) {
            self.expect(&Token::Colon)?;
            orelse = self.parse_block()?;
        }
        Ok(Stmt::If {
            test: Box::new(test),
            body,
            orelse,
        })
    }

    pub(crate) fn parse_while(&mut self) -> Result<Stmt, String> {
        self.expect(&Token::While)?;
        let test = self.parse_expr()?;
        self.expect(&Token::Colon)?;
        let body = self.parse_block()?;
        let mut orelse = Vec::new();
        if self.eat(&Token::Else) {
            self.expect(&Token::Colon)?;
            orelse = self.parse_block()?;
        }
        Ok(Stmt::While {
            test: Box::new(test),
            body,
            orelse,
        })
    }

    /// Consumes a comprehension clause's leading `for` or `async for`.
    /// Returns `Some(is_async)` if either was consumed, `None` (consuming
    /// nothing) if neither is present — callers use this both to decide
    /// whether another clause follows and to know its async-ness. `async`
    /// is a hard keyword in this lexer (never a plain identifier), so
    /// seeing it here and NOT finding `for` right after is unambiguously
    /// invalid syntax — `expect` surfaces that as a normal parse error.
    pub(crate) fn eat_comp_for(&mut self) -> Result<Option<bool>, String> {
        if self.eat(&Token::Async) {
            self.expect(&Token::For)?;
            Ok(Some(true))
        } else if self.eat(&Token::For) {
            Ok(Some(false))
        } else {
            Ok(None)
        }
    }

    /// Parse a `for` target expression, handling tuple unpacking.
    /// Track parenthesis depth so commas inside parenthesized sub-expressions
    /// (e.g., `for (a, b) in ...`) don't confuse tuple element separators.
    /// Also works for comprehension `for` clauses.
    pub(crate) fn parse_for_target_elt(&mut self) -> Result<Expr, String> {
        // `for head, *tail in ...:` — a starred sub-target collects the rest
        // of the iterable's items into a list, same as in assignment targets.
        if self.eat(&Token::Star) {
            return Ok(Expr::Starred(Box::new(self.parse_bitwise_or()?)));
        }
        self.parse_bitwise_or()
    }

    pub(crate) fn parse_for_target(&mut self) -> Result<Expr, String> {
        let mut target = self.parse_for_target_elt()?;
        if self.at(&Token::Comma) {
            let mut elts = vec![target];
            let mut paren_depth = 0usize;
            loop {
                if !self.eat(&Token::Comma) {
                    break;
                }
                if paren_depth == 0 && self.at(&Token::In) {
                    break;
                }
                // Track parenthesized expressions — commas inside ( ) don't count as tuple separators
                if self.at(&Token::LeftParen) {
                    paren_depth += 1;
                } else if self.at(&Token::RightParen) {
                    if paren_depth > 0 {
                        paren_depth -= 1;
                    }
                }
                elts.push(self.parse_for_target_elt()?);
            }
            target = Expr::Tuple(elts);
        }
        Ok(target)
    }

    pub(crate) fn parse_for(&mut self, is_async: bool) -> Result<Stmt, String> {
        self.expect(&Token::For)?;
        let target = self.parse_for_target()?;
        self.expect(&Token::In)?;
        // Parse the iterable expression — may be a comma-separated tuple without parens
        // e.g. `for x in 'a', 'b', 'c':`  (CPython accepts this syntax)
        let first_expr = if self.at(&Token::Star) {
            self.next();
            Expr::Starred(Box::new(self.parse_expr()?))
        } else {
            self.parse_expr()?
        };
        let iter = if self.eat(&Token::Comma) {
            if self.at(&Token::Colon) || self.at(&Token::Newline) || self.at(&Token::Semicolon) {
                // Single-item tuple with trailing comma: `for x in 'a',:`
                Expr::Tuple(vec![first_expr])
            } else {
                let mut elts = vec![first_expr];
                loop {
                    if self.at(&Token::Colon)
                        || self.at(&Token::Newline)
                        || self.at(&Token::Semicolon)
                        || self.at(&Token::EndOfFile)
                    {
                        break;
                    }
                    if self.at(&Token::Star) {
                        self.next();
                        elts.push(Expr::Starred(Box::new(self.parse_expr()?)));
                    } else {
                        elts.push(self.parse_expr()?);
                    }
                    if !self.eat(&Token::Comma) {
                        break;
                    }
                    if self.at(&Token::Colon)
                        || self.at(&Token::Newline)
                        || self.at(&Token::Semicolon)
                    {
                        break;
                    }
                }
                Expr::Tuple(elts)
            }
        } else {
            first_expr
        };
        self.expect(&Token::Colon)?;
        let body = self.parse_block()?;
        let mut orelse = Vec::new();
        if self.eat(&Token::Else) {
            self.expect(&Token::Colon)?;
            orelse = self.parse_block()?;
        }
        Ok(Stmt::For {
            target: Box::new(target),
            iter: Box::new(iter),
            body,
            orelse,
            is_async,
        })
    }

    pub(crate) fn parse_with(&mut self, is_async: bool) -> Result<Stmt, String> {
        self.expect(&Token::With)?;
        // PEP 617 parenthesized with-items (`with (cm1, cm2 as x):`) — see
        // `looks_like_parenthesized_with_items` for the disambiguation from
        // `(` merely starting a normal (possibly parenthesized) expression.
        let parenthesized =
            self.at(&Token::LeftParen) && self.looks_like_parenthesized_with_items();
        if parenthesized {
            self.next(); // consume the opening '('
        }
        let mut items = Vec::new();
        loop {
            let context_expr = self.parse_expr()?;
            let optional_vars = if self.eat(&Token::As) {
                Some(Box::new(self.parse_expr()?))
            } else {
                None
            };
            items.push(WithItem {
                context_expr: Box::new(context_expr),
                optional_vars,
            });
            if !self.eat(&Token::Comma) {
                break;
            }
            if parenthesized && self.at(&Token::RightParen) {
                break;
            } // trailing comma
        }
        if parenthesized {
            self.expect(&Token::RightParen)?;
        }
        self.expect(&Token::Colon)?;
        let body = self.parse_block()?;
        Ok(Stmt::With {
            items,
            body,
            is_async,
        })
    }

    pub(crate) fn parse_try(&mut self) -> Result<Stmt, String> {
        self.expect(&Token::Try)?;
        self.expect(&Token::Colon)?;
        let body = self.parse_block()?;
        let mut handlers = Vec::new();
        let mut handlers_star = Vec::new();
        let mut orelse = Vec::new();
        let mut finalbody = Vec::new();

        while self.eat(&Token::Except) {
            // Check for except* (PEP 654)
            if self.eat(&Token::Star) {
                // Mixing `except` and `except*` on the same `try` is illegal;
                // report the conflict at the position of this `except*`.
                if !handlers.is_empty() {
                    let (line, col) = self.lexer.get_line_col();
                    return Err(format!(
                        "L{}:{}: cannot have both 'except' and 'except*' on the same 'try'",
                        line, col
                    ));
                }
                let typ = if !self.at(&Token::Colon) {
                    let first = self.parse_expr()?;
                    if self.at(&Token::Comma) {
                        let mut elts = vec![first];
                        while self.eat(&Token::Comma) {
                            if self.at(&Token::Colon)
                                || self.at(&Token::Newline)
                                || self.at(&Token::Semicolon)
                            {
                                break;
                            }
                            elts.push(self.parse_expr()?);
                        }
                        Some(Box::new(Expr::Tuple(elts)))
                    } else {
                        Some(Box::new(first))
                    }
                } else {
                    None
                };
                let name = if self.eat(&Token::As) {
                    Some(self.expect_name()?)
                } else {
                    None
                };
                self.expect(&Token::Colon)?;
                let handler_body = self.parse_block()?;
                handlers_star.push(ExceptStar {
                    typ,
                    name,
                    body: handler_body,
                });
            } else {
                // Mixing `except` and `except*` on the same `try` is illegal;
                // report the conflict at the position of this `except`.
                if !handlers_star.is_empty() {
                    let (line, col) = self.lexer.get_line_col();
                    return Err(format!(
                        "L{}:{}: cannot have both 'except' and 'except*' on the same 'try'",
                        line, col
                    ));
                }
                let typ = if !self.at(&Token::Colon) {
                    let first = self.parse_expr()?;
                    // except Exc1, Exc2, ...:  (comma-separated tuple, Python 3.14+)
                    if self.at(&Token::Comma) {
                        let mut elts = vec![first];
                        while self.eat(&Token::Comma) {
                            if self.at(&Token::Colon)
                                || self.at(&Token::Newline)
                                || self.at(&Token::Semicolon)
                            {
                                break;
                            }
                            elts.push(self.parse_expr()?);
                        }
                        Some(Box::new(Expr::Tuple(elts)))
                    } else {
                        Some(Box::new(first))
                    }
                } else {
                    None
                };
                let name = if self.eat(&Token::As) {
                    Some(self.expect_name()?)
                } else {
                    None
                };
                self.expect(&Token::Colon)?;
                let handler_body = self.parse_block()?;
                handlers.push(ExceptHandler {
                    typ,
                    name,
                    body: handler_body,
                });
            }
        }

        if self.eat(&Token::Else) {
            self.expect(&Token::Colon)?;
            orelse = self.parse_block()?;
        }
        if self.eat(&Token::Finally) {
            self.expect(&Token::Colon)?;
            finalbody = self.parse_block()?;
        }
        Ok(Stmt::Try {
            body,
            handlers,
            handlers_star,
            orelse,
            finalbody,
        })
    }

    pub(crate) fn parse_match(&mut self) -> Result<Stmt, String> {
        // consume the 'match' keyword token (now Name("match"))
        self.next();
        let subject = self.parse_expr()?;
        // Match subject can be a tuple: `match x, y:`  or `match x,:`
        let subject = if self.eat(&Token::Comma) {
            let mut elts = vec![subject];
            loop {
                while self.at(&Token::Newline) {
                    self.next();
                }
                if self.at(&Token::Colon) {
                    break;
                }
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
            subject
        };
        self.expect(&Token::Colon)?;
        let cases = self.parse_match_cases()?;
        Ok(Stmt::Match {
            subject: Box::new(subject),
            cases,
        })
    }

    pub(crate) fn parse_type_alias(&mut self) -> Result<Stmt, String> {
        // consume the 'type' keyword token (now Name("type"))
        self.next();
        let name = self.expect_name()?;
        let mut type_params = Vec::new();
        // Optional: [T, U, ...] type parameters (PEP 695)
        if self.eat(&Token::LeftBracket) {
            loop {
                if self.eat(&Token::DoubleStar) {
                    let name = self.expect_name()?;
                    type_params.push(format!("**{}", name));
                } else if self.eat(&Token::Star) {
                    let name = self.expect_name()?;
                    type_params.push(format!("*{}", name));
                } else {
                    type_params.push(self.expect_name()?);
                }
                if self.eat(&Token::Colon) {
                    self.parse_expr()?; // skip bound
                }
                if self.eat(&Token::Equal) {
                    self.parse_expr()?; // skip default
                }
                if !self.eat(&Token::Comma) {
                    break;
                }
            }
            self.expect(&Token::RightBracket)?;
        }
        self.expect(&Token::Equal)?;
        let value = self.parse_expr()?;
        let _ = self.expect_newline_or_eof();
        Ok(Stmt::TypeAlias {
            name,
            type_params,
            value: Box::new(value),
        })
    }

}
