use crate::ast::*;
use crate::token::Token;

use super::Parser;

impl Parser {
    pub(crate) fn parse_import(&mut self) -> Result<Stmt, String> {
        self.next();
        let mut names = vec![self.parse_alias()?];
        while self.eat(&Token::Comma) {
            names.push(self.parse_alias()?);
        }
        let _ = self.expect_newline_or_eof();
        Ok(Stmt::Import(names))
    }

    pub(crate) fn parse_from_import(&mut self) -> Result<Stmt, String> {
        self.next();
        let level = if self.eat(&Token::Ellipsis) {
            let mut cnt = 1u32;
            while self.eat(&Token::Ellipsis) {
                cnt += 1;
            }
            Some(cnt)
        } else {
            None
        };
        let mut dots = 0u32;
        while self.eat(&Token::Dot) {
            dots += 1;
        }
        let level = level.or(if dots > 0 { Some(dots) } else { None });
        let module = if !self.at(&Token::Import) {
            Some(self.parse_dotted_name()?)
        } else {
            None
        };
        self.expect(&Token::Import)?;
        // `from __future__ import barry_as_FLUFL` enables the `<>`/`!=`
        // swap even without compiler flags (test_flufl's
        // test_barry_as_bdfl_look_ma_with_no_compiler_flags).
        if module.as_deref() == Some("__future__") && level.is_none() {
            let names = self.parse_import_names()?;
            if names.iter().any(|a| a.name == "barry_as_FLUFL") {
                self.barry_as_bdfl = true;
            }
            let _ = self.expect_newline_or_eof();
            return Ok(Stmt::ImportFrom {
                module,
                names,
                level,
            });
        }
        // Handle `from X import (a, b, c)` — parenthesized import names
        let has_paren = self.eat(&Token::LeftParen);
        if has_paren {
            while self.eat(&Token::Newline) {} // skip comment-generated newlines
        }
        let names = self.parse_import_names()?;
        if has_paren {
            while self.eat(&Token::Newline) {} // skip newlines before closing paren
            self.expect(&Token::RightParen)?;
        }
        let _ = self.expect_newline_or_eof();
        Ok(Stmt::ImportFrom {
            module,
            names,
            level,
        })
    }

    pub(crate) fn parse_dotted_name(&mut self) -> Result<String, String> {
        let mut name = self.expect_name()?;
        while self.eat(&Token::Dot) {
            name.push('.');
            name.push_str(&self.expect_name()?);
        }
        Ok(name)
    }

    pub(crate) fn parse_alias(&mut self) -> Result<Alias, String> {
        let name = self.parse_dotted_name()?;
        let asname = if self.eat(&Token::As) {
            Some(self.expect_name()?)
        } else {
            None
        };
        Ok(Alias { name, asname })
    }

    pub(crate) fn parse_import_names(&mut self) -> Result<Vec<Alias>, String> {
        if self.at(&Token::Star) {
            self.next();
            return Ok(vec![Alias {
                name: "*".to_string(),
                asname: None,
            }]);
        }
        let mut names = vec![self.parse_alias()?];
        while self.eat(&Token::Comma) {
            while self.eat(&Token::Newline) {} // skip newlines between items
            if self.at(&Token::RightParen) {
                break;
            } // trailing comma
            if self.at(&Token::Star) {
                names.push(Alias {
                    name: "*".to_string(),
                    asname: None,
                });
                self.next();
                break;
            }
            names.push(self.parse_alias()?);
        }
        Ok(names)
    }
}
