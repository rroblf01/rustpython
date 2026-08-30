use crate::ast::*;
use crate::token::Token;

use super::Parser;

impl Parser {
    pub(crate) fn parse_primary(&mut self) -> Result<Expr, String> {
        let mut expr = self.parse_atom()?;
        loop {
            if self.eat(&Token::Dot) {
                let attr = self.expect_name()?;
                expr = Expr::Attribute {
                    value: Box::new(expr),
                    attr,
                };
            } else if self.eat(&Token::LeftParen) {
                let mut args = Vec::new();
                let mut keywords = Vec::new();
                let mut seen_keyword = false;
                let mut seen_double_star = false;
                let mut seen_kw_names: std::collections::HashSet<String> =
                    std::collections::HashSet::new();
                if !self.at(&Token::RightParen) {
                    loop {
                        if self.at(&Token::RightParen) {
                            break;
                        }
                        if self.at(&Token::Star) {
                            // `f(k=1, *args)` is VALID Python (test_print's
                            // dispatch lambdas do exactly this) — only a
                            // POSITIONAL arg after a keyword is an error, and
                            // a duplicate keyword is caught below.
                            if seen_double_star {
                                return Err(
                                    "iterable argument unpacking follows keyword argument unpacking"
                                        .to_string(),
                                );
                            }
                            self.next(); // consume *
                            let starred = self.parse_expr()?;
                            args.push(Expr::Starred(Box::new(starred)));
                        } else if self.at(&Token::DoubleStar) {
                            // Multiple `**kwargs` unpackings are valid
                            // (`f(**a, **b)` merges them) — only a
                            // positional/`*args`/`k=v` after a `**` is an
                            // error, handled in the branches below.
                            self.next();
                            let value = self.parse_expr()?;
                            keywords.push(Keyword {
                                arg: None,
                                value: Box::new(value),
                            });
                            seen_keyword = true;
                            seen_double_star = true;
                        } else if self.peek() == &Token::Equal
                            && (matches!(&self.current, Token::Name(_))
                                || self.at(&Token::Underscore))
                        {
                            let arg = Some(if self.eat(&Token::Underscore) {
                                "_".to_string()
                            } else {
                                self.expect_name()?
                            });
                            if let Some(name) = &arg {
                                if !seen_kw_names.insert(name.clone()) {
                                    return Err(format!("keyword argument repeated: {}", name));
                                }
                            }
                            self.expect(&Token::Equal)?;
                            let value = self.parse_expr()?;
                            keywords.push(Keyword {
                                arg,
                                value: Box::new(value),
                            });
                            seen_keyword = true;
                        } else {
                            if seen_double_star {
                                return Err(
                                    "positional argument follows keyword argument unpacking"
                                        .to_string(),
                                );
                            }
                            if seen_keyword {
                                return Err(
                                    "positional argument follows keyword argument".to_string()
                                );
                            }
                            // Parse expression with full ternary support
                            let mut expr = self.parse_conditional_expr()?;
                            // Walrus operator as a call argument: f(x := expr)
                            if self.eat(&Token::Walrus) {
                                let value = self.parse_expr()?;
                                expr = Expr::NamedExpr {
                                    target: Box::new(expr),
                                    value: Box::new(value),
                                };
                            }
                            // Check for generator expression as sole argument: f(x for x in lst)
                            if (self.at(&Token::For) || self.at(&Token::Async))
                                && args.is_empty()
                                && keywords.is_empty()
                            {
                                self.genexpr_depth += 1;
                                let is_async = self.eat_comp_for()?.unwrap();
                                let target = self.parse_for_target()?;
                                self.expect(&Token::In)?;
                                let iter = self.parse_or_expr()?;
                                let mut generators = vec![Comprehension {
                                    target: Box::new(target),
                                    iter: Box::new(iter),
                                    ifs: Vec::new(),
                                    is_async,
                                }];
                                while let Some(is_async) = self.eat_comp_for()? {
                                    let t = self.parse_for_target()?;
                                    self.expect(&Token::In)?;
                                    let i = self.parse_or_expr()?;
                                    generators.push(Comprehension {
                                        target: Box::new(t),
                                        iter: Box::new(i),
                                        ifs: Vec::new(),
                                        is_async,
                                    });
                                }
                                if self.eat(&Token::If) {
                                    if let Some(last) = generators.last_mut() {
                                        last.ifs.push(self.parse_or_expr()?);
                                        while self.eat(&Token::If) {
                                            last.ifs.push(self.parse_or_expr()?);
                                        }
                                    }
                                }
                                while let Some(is_async) = self.eat_comp_for()? {
                                    let t = self.parse_for_target()?;
                                    self.expect(&Token::In)?;
                                    let i = self.parse_or_expr()?;
                                    generators.push(Comprehension {
                                        target: Box::new(t),
                                        iter: Box::new(i),
                                        ifs: Vec::new(),
                                        is_async,
                                    });
                                    if self.eat(&Token::If) {
                                        if let Some(last) = generators.last_mut() {
                                            last.ifs.push(self.parse_or_expr()?);
                                            while self.eat(&Token::If) {
                                                last.ifs.push(self.parse_or_expr()?);
                                            }
                                        }
                                    }
                                }
                                args.push(Expr::GeneratorExp {
                                    elt: Box::new(expr),
                                    generators,
                                });
                                self.genexpr_depth -= 1;
                                if !self.eat(&Token::Comma) {
                                    break;
                                }
                                continue;
                            }
                            args.push(expr);
                        }
                        if !self.eat(&Token::Comma) {
                            break;
                        }
                        while self.eat(&Token::Newline) {} // skip comment-generated newlines
                    }
                }
                self.expect(&Token::RightParen)?;
                expr = Expr::Call {
                    func: Box::new(expr),
                    args,
                    keywords,
                };
            } else if self.eat(&Token::LeftBracket) {
                let slice = self.parse_slice_or_expr()?;
                self.expect(&Token::RightBracket)?;
                expr = Expr::Subscript {
                    value: Box::new(expr),
                    slice: Box::new(slice),
                };
            } else {
                break;
            }
        }
        Ok(expr)
    }

    pub(crate) fn parse_slice_or_expr(&mut self) -> Result<Expr, String> {
        if self.eat(&Token::Colon) {
            let mut upper = None;
            let step;
            // Check for ::
            if self.eat(&Token::Colon) {
                step = if !self.at(&Token::RightBracket) && !self.at(&Token::Comma) {
                    Some(Box::new(self.parse_expr()?))
                } else {
                    None
                };
            } else {
                upper = if !self.at(&Token::RightBracket) && !self.at(&Token::Comma) {
                    Some(Box::new(self.parse_expr()?))
                } else {
                    None
                };
                step = if self.eat(&Token::Colon) {
                    if !self.at(&Token::RightBracket) && !self.at(&Token::Comma) {
                        Some(Box::new(self.parse_expr()?))
                    } else {
                        None
                    }
                } else {
                    None
                };
            }
            let slice = Expr::Slice {
                lower: None,
                upper,
                step,
            };
            if self.eat(&Token::Comma) {
                let mut elts = vec![slice];
                loop {
                    while self.at(&Token::Newline) {
                        self.next();
                    }
                    if self.at(&Token::RightBracket) {
                        break;
                    }
                    if self.at(&Token::Colon) {
                        self.next();
                        let u = if !self.at(&Token::RightBracket)
                            && !self.at(&Token::Comma)
                            && !self.at(&Token::Colon)
                        {
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
                            lower: None,
                            upper: u,
                            step: s,
                        });
                    } else {
                        let e = self.parse_expr()?;
                        if self.eat(&Token::Colon) {
                            let u = if !self.at(&Token::RightBracket)
                                && !self.at(&Token::Comma)
                                && !self.at(&Token::Colon)
                            {
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
                    if !self.eat(&Token::Comma) {
                        break;
                    }
                }
                return Ok(Expr::Tuple(elts));
            }
            return Ok(slice);
        }
        // Expression-first path
        let lower = if self.eat(&Token::Star) {
            Expr::Starred(Box::new(self.parse_expr()?))
        } else {
            self.parse_expr()?
        };
        // Handle comma-separated expressions in subscript: X[a, b, c]
        if self.eat(&Token::Comma) {
            let mut elts = vec![lower];
            loop {
                while self.at(&Token::Newline) {
                    self.next();
                }
                if self.at(&Token::RightBracket) {
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
            return Ok(Expr::Tuple(elts));
        }
        if self.eat(&Token::Colon) {
            // The upper bound is empty not just when the slice ends
            // (`]`) or another subscript element follows (`,`), but also
            // when the step colon immediately follows (`lower::step`,
            // e.g. `a[1::2]`) — without checking for Colon here too, this
            // tried to parse an expression starting at that second `:`
            // and failed with "Expected expression, got Colon".
            let upper = if !self.at(&Token::RightBracket)
                && !self.at(&Token::Comma)
                && !self.at(&Token::Colon)
            {
                Some(Box::new(self.parse_expr()?))
            } else {
                None
            };
            let step = if self.eat(&Token::Colon) {
                if !self.at(&Token::RightBracket) && !self.at(&Token::Comma) {
                    Some(Box::new(self.parse_expr()?))
                } else {
                    None
                }
            } else {
                None
            };
            // After a slice, check for comma (multi-dim slice or tuple in subscript).
            // If found, collect remaining comma-separated expressions into a tuple.
            if self.eat(&Token::Comma) {
                let mut elts = vec![Expr::Slice {
                    lower: Some(Box::new(lower)),
                    upper,
                    step,
                }];
                loop {
                    if self.at(&Token::RightBracket) {
                        break;
                    }
                    // Each element may itself be a slice (a:b)
                    if self.eat(&Token::Colon) {
                        let u = if !self.at(&Token::RightBracket)
                            && !self.at(&Token::Comma)
                            && !self.at(&Token::Colon)
                        {
                            Some(Box::new(self.parse_expr()?))
                        } else {
                            None
                        };
                        let s = if self.eat(&Token::Colon) {
                            if !self.at(&Token::RightBracket)
                                && !self.at(&Token::Comma)
                                && !self.at(&Token::Colon)
                            {
                                Some(Box::new(self.parse_expr()?))
                            } else {
                                None
                            }
                        } else {
                            None
                        };
                        elts.push(Expr::Slice {
                            lower: None,
                            upper: u,
                            step: s,
                        });
                    } else if self.eat(&Token::Star) {
                        elts.push(Expr::Starred(Box::new(self.parse_expr()?)));
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
                    if !self.eat(&Token::Comma) {
                        break;
                    }
                }
                return Ok(Expr::Tuple(elts));
            }
            Ok(Expr::Slice {
                lower: Some(Box::new(lower)),
                upper,
                step,
            })
        } else {
            Ok(lower)
        }
    }


}
