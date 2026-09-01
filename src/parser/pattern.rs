use crate::ast::*;
use crate::token::Token;

use super::Parser;

impl Parser {
    pub(crate) fn parse_match_cases(&mut self) -> Result<Vec<MatchCase>, String> {
        // Grammar: 'match' subject ':' NEWLINE INDENT case_block+ DEDENT —
        // there are TWO nested indentation levels here: this outer one
        // (wrapping the whole set of `case` clauses) and, separately, each
        // case's own body (opened/closed by parse_block() below, which
        // already consumes exactly its own one closing Dedent). Previously
        // this loop ate Indent/Dedent tokens unconditionally while looking
        // for the next `case`, which — once the last case's body ended —
        // also swallowed the Dedent(s) belonging to whatever *encloses* the
        // match statement (the function/module body), silently truncating
        // everything after it from the parse tree.
        let mut cases = Vec::new();
        self.eat(&Token::Newline);
        let had_indent = self.eat(&Token::Indent);
        loop {
            while self.eat(&Token::Newline) {}
            if !matches!(&self.current, Token::Name(n) if n == "case") {
                break;
            }
            self.next(); // consume "case" keyword
            let mut pattern = self.parse_pattern()?;
            // Open sequence pattern: `case 0, *x:`  (comma-separated without parens)
            if self.eat(&Token::Comma) {
                let mut patterns = vec![pattern];
                loop {
                    while self.at(&Token::Newline) {
                        self.next();
                    }
                    if self.at(&Token::Colon) || self.at(&Token::If) {
                        break;
                    }
                    if self.eat(&Token::Star) {
                        let name = if matches!(&self.current, Token::Name(_)) {
                            Some(self.expect_name()?)
                        } else if self.eat(&Token::Underscore) {
                            Some("_".to_string())
                        } else {
                            None
                        };
                        patterns.push(Pattern::MatchStar { name });
                    } else {
                        patterns.push(self.parse_pattern()?);
                    }
                    if !self.eat(&Token::Comma) {
                        break;
                    }
                }
                pattern = Pattern::MatchSequence(patterns);
            }
            let guard = if self.eat(&Token::If) {
                Some(Box::new(self.parse_expr()?))
            } else {
                None
            };
            self.expect(&Token::Colon)?;
            let body = self.parse_block()?;
            cases.push(MatchCase {
                pattern,
                guard,
                body,
            });
        }
        if had_indent {
            // Exactly one Dedent, matching the Indent consumed above — any
            // further dedents belong to the enclosing scope.
            self.eat(&Token::Dedent);
        }
        Ok(cases)
    }

    pub(crate) fn parse_pattern(&mut self) -> Result<Pattern, String> {
        self.parse_or_pattern()
    }

    pub(crate) fn parse_or_pattern(&mut self) -> Result<Pattern, String> {
        let mut patterns = vec![self.parse_as_pattern()?];
        while self.eat(&Token::Pipe) {
            patterns.push(self.parse_as_pattern()?);
        }
        if patterns.len() == 1 {
            Ok(patterns.into_iter().next().unwrap())
        } else {
            Ok(Pattern::MatchOr(patterns))
        }
    }

    pub(crate) fn parse_as_pattern(&mut self) -> Result<Pattern, String> {
        let pattern = self.parse_literal_pattern()?;
        if self.eat(&Token::As) {
            let name = Some(self.expect_name()?);
            Ok(Pattern::MatchAs {
                pattern: Some(Box::new(pattern)),
                name,
            })
        } else {
            Ok(pattern)
        }
    }

    pub(crate) fn parse_literal_pattern(&mut self) -> Result<Pattern, String> {
        if self.at(&Token::Underscore) {
            self.next();
            return Ok(Pattern::MatchAs {
                pattern: None,
                name: None,
            });
        }
        if self.eat(&Token::Star) {
            let name = if matches!(&self.current, Token::Name(_)) {
                Some(self.expect_name()?)
            } else if self.eat(&Token::Underscore) {
                Some("_".to_string())
            } else {
                None
            };
            return Ok(Pattern::MatchStar { name });
        }
        if matches!(&self.current, Token::Name(_)) {
            let name = self.expect_name()?;
            if self.at(&Token::LeftParen) {
                return self.parse_class_pattern(name);
            }
            // A dotted name (`case Format.STRING:`, `case some.deep.CONST:`)
            // is a "value pattern" — matched by equality against the
            // referenced value — NOT a capture pattern, per real Python's
            // match-statement grammar (only a bare, undotted name captures).
            // Without this check every qualified-constant case clause
            // (extremely common — enum members, module-level constants)
            // was silently treated as an always-matching capture binding
            // instead of an equality check. Real code: CPython 3.14's own
            // `annotationlib.py`, `case Format.STRING:` inside a real
            // stdlib function.
            if self.at(&Token::Dot) {
                let mut expr = Expr::Name(name);
                while self.eat(&Token::Dot) {
                    let attr = self.expect_name()?;
                    expr = Expr::Attribute {
                        value: Box::new(expr),
                        attr,
                    };
                }
                return Ok(Pattern::MatchValue(Box::new(expr)));
            }
            if name == "_" {
                return Ok(Pattern::MatchAs {
                    pattern: None,
                    name: None,
                });
            }
            return Ok(Pattern::MatchAs {
                pattern: None,
                name: Some(name),
            });
        }
        if self.at(&Token::LeftParen) || self.at(&Token::LeftBracket) {
            return self.parse_sequence_pattern();
        }
        if self.at(&Token::LeftBrace) {
            return self.parse_mapping_pattern();
        }
        // A literal/value pattern (`case 1:`, `case -1:`, `case 1+2j:`,
        // `case "x":`) must NOT consume `|` here — `|` inside a `case`
        // clause is ALWAYS the or-pattern separator (`parse_or_pattern`'s
        // own job, one level up), never a bitwise-or expression. This used
        // to call `parse_or_expr` (the full expression grammar, all the way
        // up through logical `or`/`and`/comparison/bitwise-or), which
        // greedily swallowed the `|` as `Expr::BinOp` before
        // `parse_or_pattern`'s `while self.eat(&Token::Pipe)` loop ever saw
        // it — so `case 'Y' | 'y' | 'G':` compiled as ONE `MatchValue`
        // wrapping the literal expression `'Y' | 'y' | 'G'`, evaluated at
        // MATCH TIME as a real binary `|` between string values (raising
        // `TypeError: unsupported operand type(s) for |: 'str' and 'str'`
        // the instant any such case ran) instead of three separate
        // alternatives. Confirmed general, not `_strptime.py`-specific: any
        // `case A | B:` where `A`/`B` don't happen to support `|` (almost
        // everything except numbers) was completely broken. Stopping at
        // `parse_bitwise_xor` (one precedence level tighter, skipping
        // `parse_bitwise_or`, logical `or`/`and`/`not`, and comparisons —
        // none of which real Python's own literal-pattern grammar permits
        // anyway) still correctly parses negative numbers and complex
        // literals (`-5`, `1+2j`), just without swallowing a trailing `|`.
        let expr = self.parse_bitwise_xor()?;
        Ok(Pattern::MatchValue(Box::new(expr)))
    }

    pub(crate) fn parse_class_pattern(&mut self, name: String) -> Result<Pattern, String> {
        self.expect(&Token::LeftParen)?;
        let mut patterns = Vec::new();
        let mut kwd_attrs = Vec::new();
        let mut kwd_patterns = Vec::new();
        if !self.at(&Token::RightParen) {
            loop {
                if matches!(&self.current, Token::Name(_)) && self.peek() == &Token::Equal {
                    kwd_attrs.push(self.expect_name()?);
                    self.expect(&Token::Equal)?;
                    kwd_patterns.push(self.parse_pattern()?);
                } else {
                    patterns.push(self.parse_pattern()?);
                }
                if !self.eat(&Token::Comma) {
                    break;
                }
            }
        }
        self.expect(&Token::RightParen)?;
        let cls = Expr::Name(name);
        Ok(Pattern::MatchClass {
            cls: Box::new(cls),
            patterns,
            kwd_attrs,
            kwd_patterns,
        })
    }

    pub(crate) fn parse_sequence_pattern(&mut self) -> Result<Pattern, String> {
        let open = if self.eat(&Token::LeftBracket) {
            "["
        } else {
            self.expect(&Token::LeftParen)?;
            "("
        };
        let mut patterns = Vec::new();
        if open == "(" && self.at(&Token::RightParen) {
            self.next();
            return Ok(Pattern::MatchSequence(patterns));
        }
        loop {
            if open == "[" && self.at(&Token::RightBracket) {
                break;
            }
            if open == "(" && self.at(&Token::RightParen) {
                break;
            }
            if self.eat(&Token::Star) {
                let name = if matches!(&self.current, Token::Name(_)) {
                    Some(self.expect_name()?)
                } else if self.eat(&Token::Underscore) {
                    Some("_".to_string())
                } else {
                    None
                };
                patterns.push(Pattern::MatchStar { name });
            } else {
                patterns.push(self.parse_pattern()?);
            }
            if !self.eat(&Token::Comma) {
                break;
            }
        }
        let close = if open == "[" {
            Token::RightBracket
        } else {
            Token::RightParen
        };
        self.expect(&close)?;
        Ok(Pattern::MatchSequence(patterns))
    }

    pub(crate) fn parse_mapping_pattern(&mut self) -> Result<Pattern, String> {
        self.expect(&Token::LeftBrace)?;
        let mut keys = Vec::new();
        let mut rest = None;
        if !self.at(&Token::RightBrace) {
            loop {
                if self.eat(&Token::DoubleStar) {
                    rest = Some(self.expect_name()?);
                    if !self.eat(&Token::Comma) {
                        break;
                    }
                    continue;
                }
                keys.push(self.parse_literal_pattern()?);
                self.expect(&Token::Colon)?;
                keys.push(self.parse_pattern()?);
                if !self.eat(&Token::Comma) {
                    break;
                }
            }
        }
        self.expect(&Token::RightBrace)?;
        Ok(Pattern::MatchMapping { keys, rest })
    }
}
