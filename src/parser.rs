use crate::ast::*;
use crate::token::{Lexer, Token};

pub struct Parser {
    lexer: Lexer,
    current: Token,
    peeked: Option<Token>,
}

macro_rules! unexpected_token {
    ($self:expr, $expected:expr) => {
        Err(format!(
            "L{}:{}: Expected {}, got {}",
            $self.lexer.get_line_col().0,
            $self.lexer.get_line_col().1,
            $expected,
            $self.current
        ))
    };
}

impl Parser {
    pub fn new(source: &str) -> Self {
        let mut lexer = Lexer::new(source);
        let first = lexer.next_token();
        Parser {
            lexer,
            current: first,
            peeked: None,
        }
    }

    fn next(&mut self) -> Token {
        let tok = self.peeked.take().unwrap_or_else(|| self.lexer.next_token());
        std::mem::replace(&mut self.current, tok)
    }

    fn peek(&mut self) -> &Token {
        if self.peeked.is_none() {
            self.peeked = Some(self.lexer.next_token());
        }
        self.peeked.as_ref().unwrap()
    }

    fn at(&self, tok: &Token) -> bool {
        self.current == *tok
    }

    fn eat(&mut self, tok: &Token) -> bool {
        if self.at(tok) {
            self.next();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, tok: &Token) -> Result<(), String> {
        if self.eat(tok) {
            Ok(())
        } else {
            unexpected_token!(self, tok)
        }
    }

    fn expect_name(&mut self) -> Result<String, String> {
        match &self.current {
            Token::Name(s) => {
                let s = s.clone();
                self.next();
                Ok(s)
            }
            _ => unexpected_token!(self, "NAME"),
        }
    }

    // ---- Program ----

    pub fn parse_program(&mut self) -> Result<Program, String> {
        let mut stmts = Vec::new();
        while !self.at(&Token::EndOfFile) {
            while self.at(&Token::Newline) || self.at(&Token::Indent) || self.at(&Token::Dedent) {
                self.next();
            }
            if self.at(&Token::EndOfFile) {
                break;
            }
            stmts.push(self.parse_stmt()?);
        }
        Ok(Program::Module(stmts))
    }

    // ---- Statements ----

    fn parse_stmt(&mut self) -> Result<Stmt, String> {
        self.parse_simple_stmt()
    }

    fn parse_simple_stmt(&mut self) -> Result<Stmt, String> {
        if self.at(&Token::Pass) {
            self.next();
            self.expect_newline_or_eof();
            return Ok(Stmt::Pass);
        }
        if self.at(&Token::Break) {
            self.next();
            self.expect_newline_or_eof();
            return Ok(Stmt::Break);
        }
        if self.at(&Token::Continue) {
            self.next();
            self.expect_newline_or_eof();
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
            return self.parse_for();
        }
        if self.at(&Token::With) {
            return self.parse_with();
        }
        if self.at(&Token::Try) {
            return self.parse_try();
        }
        if self.at(&Token::Match) {
            return self.parse_match();
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

        let expr = self.parse_expr()?;
        self.parse_stmt_tail(expr)
    }

    fn parse_stmt_tail(&mut self, expr: Expr) -> Result<Stmt, String> {
        if self.eat(&Token::Equal) {
            let value = self.parse_expr()?;
            let mut targets = vec![expr];
            while self.eat(&Token::Equal) {
                targets.push(self.parse_expr()?);
            }
            self.expect_newline_or_eof();
            Ok(Stmt::Assign { targets, value: Box::new(value) })
        } else if self.at(&Token::PlusEqual) || self.at(&Token::MinusEqual)
            || self.at(&Token::StarEqual) || self.at(&Token::SlashEqual)
            || self.at(&Token::DoubleStarEqual) || self.at(&Token::DoubleSlashEqual)
            || self.at(&Token::PercentEqual) || self.at(&Token::PipeEqual)
            || self.at(&Token::AmpersandEqual) || self.at(&Token::CaretEqual)
            || self.at(&Token::LeftShiftEqual) || self.at(&Token::RightShiftEqual)
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
            let value = self.parse_expr()?;
            self.expect_newline_or_eof();
            Ok(Stmt::AugAssign {
                target: Box::new(expr),
                op,
                value: Box::new(value),
            })
        } else {
            self.expect_newline_or_eof();
            Ok(Stmt::Expr(Box::new(expr)))
        }
    }

    fn expect_newline_or_eof(&mut self) -> Result<(), String> {
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
        Ok(())
    }

    // ---- Compound statements ----

    fn parse_function_def(&mut self) -> Result<Stmt, String> {
        let decorator_list = Vec::new();
        let async_token = self.at(&Token::Async);
        if async_token {
            self.next(); // async
        }
        self.expect(&Token::Def)?;
        let name = self.expect_name()?;
        self.expect(&Token::LeftParen)?;
        let args = self.parse_args()?;
        self.expect(&Token::RightParen)?;
        let returns = if self.eat(&Token::Arrow) {
            Some(Box::new(self.parse_expr()?))
        } else {
            None
        };
        self.expect(&Token::Colon)?;
        let body = self.parse_block()?;
        Ok(Stmt::FunctionDef {
            name,
            args,
            body,
            decorator_list,
            returns,
            is_async: async_token,
        })
    }

    fn parse_decorated(&mut self) -> Result<Stmt, String> {
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
            Stmt::FunctionDef { decorator_list: d, .. }
            | Stmt::ClassDef { decorator_list: d, .. } => {
                *d = decorator_list;
            }
            _ => return Err("Decorator on non-function/class".to_string()),
        }
        Ok(stmt)
    }

    fn parse_class(&mut self) -> Result<Stmt, String> {
        self.expect(&Token::Class)?;
        let name = self.expect_name()?;
        let mut bases = Vec::new();
        let mut keywords = Vec::new();
        if self.eat(&Token::LeftParen) {
            if !self.at(&Token::RightParen) {
                loop {
                    if matches!(&self.current, Token::Name(_)) && self.peek() == &Token::Equal {
                        let arg = Some(self.expect_name()?);
                        self.expect(&Token::Equal)?;
                        let value = self.parse_expr()?;
                        keywords.push(Keyword { arg, value: Box::new(value) });
                    } else {
                        bases.push(self.parse_expr()?);
                    }
                    if !self.eat(&Token::Comma) { break; }
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
        })
    }

    fn parse_if(&mut self) -> Result<Stmt, String> {
        self.expect(&Token::If)?;
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

    fn parse_if_elif(&mut self) -> Result<Stmt, String> {
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

    fn parse_while(&mut self) -> Result<Stmt, String> {
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

    fn parse_for(&mut self) -> Result<Stmt, String> {
        self.expect(&Token::For)?;
        let target = self.parse_bitwise_or()?;
        self.expect(&Token::In)?;
        let iter = self.parse_expr()?;
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
        })
    }

    fn parse_with(&mut self) -> Result<Stmt, String> {
        self.expect(&Token::With)?;
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
            if !self.eat(&Token::Comma) { break; }
        }
        self.expect(&Token::Colon)?;
        let body = self.parse_block()?;
        Ok(Stmt::With { items, body })
    }

    fn parse_try(&mut self) -> Result<Stmt, String> {
        self.expect(&Token::Try)?;
        self.expect(&Token::Colon)?;
        let body = self.parse_block()?;
        let mut handlers = Vec::new();
        let mut orelse = Vec::new();
        let mut finalbody = Vec::new();

        while self.eat(&Token::Except) {
            let typ = if !self.at(&Token::Colon) {
                Some(Box::new(self.parse_expr()?))
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

        if self.eat(&Token::Else) {
            self.expect(&Token::Colon)?;
            orelse = self.parse_block()?;
        }
        if self.eat(&Token::Finally) {
            self.expect(&Token::Colon)?;
            finalbody = self.parse_block()?;
        }
        Ok(Stmt::Try { body, handlers, orelse, finalbody })
    }

    fn parse_match(&mut self) -> Result<Stmt, String> {
        self.expect(&Token::Match)?;
        let subject = self.parse_expr()?;
        self.expect(&Token::Colon)?;
        let cases = self.parse_match_cases()?;
        Ok(Stmt::Match {
            subject: Box::new(subject),
            cases,
        })
    }

    fn parse_match_cases(&mut self) -> Result<Vec<MatchCase>, String> {
        let mut cases = Vec::new();
        loop {
            while self.eat(&Token::Newline) || self.eat(&Token::Indent) || self.eat(&Token::Dedent) {}
            if !self.eat(&Token::Case) {
                break;
            }
            let pattern = self.parse_pattern()?;
            let guard = if self.eat(&Token::If) {
                Some(Box::new(self.parse_expr()?))
            } else {
                None
            };
            self.expect(&Token::Colon)?;
            let body = self.parse_block()?;
            cases.push(MatchCase { pattern, guard, body });
        }
        Ok(cases)
    }

    fn parse_pattern(&mut self) -> Result<Pattern, String> {
        self.parse_or_pattern()
    }

    fn parse_or_pattern(&mut self) -> Result<Pattern, String> {
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

    fn parse_as_pattern(&mut self) -> Result<Pattern, String> {
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

    fn parse_literal_pattern(&mut self) -> Result<Pattern, String> {
        if self.at(&Token::Underscore) {
            self.next();
            return Ok(Pattern::MatchAs { pattern: None, name: None });
        }
        if matches!(&self.current, Token::Name(_)) {
            let name = self.expect_name()?;
            if self.at(&Token::LeftParen) {
                return self.parse_class_pattern(name);
            }
            if name == "_" {
                return Ok(Pattern::MatchAs { pattern: None, name: None });
            }
            return Ok(Pattern::MatchAs { pattern: None, name: Some(name) });
        }
        if self.at(&Token::LeftParen) || self.at(&Token::LeftBracket) {
            return self.parse_sequence_pattern();
        }
        if self.at(&Token::LeftBrace) {
            return self.parse_mapping_pattern();
        }
        let expr = self.parse_expr()?;
        Ok(Pattern::MatchValue(Box::new(expr)))
    }

    fn parse_class_pattern(&mut self, name: String) -> Result<Pattern, String> {
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
                if !self.eat(&Token::Comma) { break; }
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

    fn parse_sequence_pattern(&mut self) -> Result<Pattern, String> {
        let open = if self.eat(&Token::LeftBracket) { "[" } else { self.expect(&Token::LeftParen)?; "(" };
        let mut patterns = Vec::new();
        if open == "(" && self.at(&Token::RightParen) {
            self.next();
            return Ok(Pattern::MatchSequence(patterns));
        }
        loop {
            if open == "[" && self.at(&Token::RightBracket) {
                break;
            }
            if self.eat(&Token::Star) {
                let name = if matches!(&self.current, Token::Name(_)) {
                    Some(self.expect_name()?)
                } else {
                    None
                };
                patterns.push(Pattern::MatchStar { name });
            } else {
                patterns.push(self.parse_pattern()?);
            }
            if !self.eat(&Token::Comma) { break; }
        }
        let close = if open == "[" { Token::RightBracket } else { Token::RightParen };
        self.expect(&close)?;
        Ok(Pattern::MatchSequence(patterns))
    }

    fn parse_mapping_pattern(&mut self) -> Result<Pattern, String> {
        self.expect(&Token::LeftBrace)?;
        let mut keys = Vec::new();
        let mut rest = None;
        if !self.at(&Token::RightBrace) {
            loop {
                if self.eat(&Token::DoubleStar) {
                    rest = Some(self.expect_name()?);
                    if !self.eat(&Token::Comma) { break; }
                    continue;
                }
                keys.push(self.parse_literal_pattern()?);
                self.expect(&Token::Colon)?;
                keys.push(self.parse_pattern()?);
                if !self.eat(&Token::Comma) { break; }
            }
        }
        self.expect(&Token::RightBrace)?;
        Ok(Pattern::MatchMapping { keys, rest })
    }

    fn skip_over_blank_lines(&mut self) {
        while self.eat(&Token::Newline) {}
    }

    // ---- Other statements ----

    fn parse_return(&mut self) -> Result<Stmt, String> {
        self.expect(&Token::Return)?;
        let value = if !self.at(&Token::Newline) && !self.at(&Token::Semicolon) && !self.at(&Token::EndOfFile) {
            Some(Box::new(self.parse_expr()?))
        } else {
            None
        };
        self.expect_newline_or_eof();
        Ok(Stmt::Return(value))
    }

    fn parse_yield_stmt(&mut self) -> Result<Stmt, String> {
        let expr = self.parse_yield_expr()?;
        self.expect_newline_or_eof();
        Ok(Stmt::Expr(Box::new(expr)))
    }

    fn parse_raise(&mut self) -> Result<Stmt, String> {
        self.next();
        let exc = if !self.at(&Token::Newline) && !self.at(&Token::EndOfFile) {
            let e = self.parse_expr()?;
            if self.eat(&Token::From) {
                let cause = self.parse_expr()?;
                self.expect_newline_or_eof();
                return Ok(Stmt::Raise {
                    exc: Some(Box::new(e)),
                    cause: Some(Box::new(cause)),
                });
            }
            Some(Box::new(e))
        } else {
            None
        };
        self.expect_newline_or_eof();
        Ok(Stmt::Raise { exc, cause: None })
    }

    fn parse_global(&mut self) -> Result<Stmt, String> {
        self.next();
        let mut names = vec![self.expect_name()?];
        while self.eat(&Token::Comma) {
            names.push(self.expect_name()?);
        }
        self.expect_newline_or_eof();
        Ok(Stmt::Global(names))
    }

    fn parse_nonlocal(&mut self) -> Result<Stmt, String> {
        self.next();
        let mut names = vec![self.expect_name()?];
        while self.eat(&Token::Comma) {
            names.push(self.expect_name()?);
        }
        self.expect_newline_or_eof();
        Ok(Stmt::Nonlocal(names))
    }

    fn parse_assert(&mut self) -> Result<Stmt, String> {
        self.next();
        let test = self.parse_expr()?;
        let msg = if self.eat(&Token::Comma) {
            Some(Box::new(self.parse_expr()?))
        } else {
            None
        };
        self.expect_newline_or_eof();
        Ok(Stmt::Assert { test: Box::new(test), msg })
    }

    fn parse_del(&mut self) -> Result<Stmt, String> {
        self.next();
        let mut targets = vec![self.parse_expr()?];
        while self.eat(&Token::Comma) {
            targets.push(self.parse_expr()?);
        }
        self.expect_newline_or_eof();
        Ok(Stmt::Delete(targets))
    }

    fn parse_import(&mut self) -> Result<Stmt, String> {
        self.next();
        let mut names = vec![self.parse_alias()?];
        while self.eat(&Token::Comma) {
            names.push(self.parse_alias()?);
        }
        self.expect_newline_or_eof();
        Ok(Stmt::Import(names))
    }

    fn parse_from_import(&mut self) -> Result<Stmt, String> {
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
        let names = self.parse_import_names()?;
        self.expect_newline_or_eof();
        Ok(Stmt::ImportFrom { module, names, level })
    }

    fn parse_dotted_name(&mut self) -> Result<String, String> {
        let mut name = self.expect_name()?;
        while self.eat(&Token::Dot) {
            name.push('.');
            name.push_str(&self.expect_name()?);
        }
        Ok(name)
    }

    fn parse_alias(&mut self) -> Result<Alias, String> {
        let name = self.parse_dotted_name()?;
        let asname = if self.eat(&Token::As) {
            Some(self.expect_name()?)
        } else {
            None
        };
        Ok(Alias { name, asname })
    }

    fn parse_import_names(&mut self) -> Result<Vec<Alias>, String> {
        if self.at(&Token::Star) {
            self.next();
            return Ok(vec![Alias { name: "*".to_string(), asname: None }]);
        }
        let mut names = vec![self.parse_alias()?];
        while self.eat(&Token::Comma) {
            if self.at(&Token::Star) {
                names.push(Alias { name: "*".to_string(), asname: None });
                self.next();
                break;
            }
            names.push(self.parse_alias()?);
        }
        Ok(names)
    }

    fn parse_args(&mut self) -> Result<Vec<Arg>, String> {
        let mut args = Vec::new();
        if !self.at(&Token::RightParen) {
            loop {
                if self.eat(&Token::DoubleStar) {
                    let name = self.expect_name()?;
                    let annotation = if self.eat(&Token::Colon) {
                        Some(Box::new(self.parse_expr()?))
                    } else { None };
                    args.push(Arg { arg: name, annotation, is_vararg: false, is_kwarg: true, default: None });
                    if !self.eat(&Token::Comma) { break; }
                } else if self.eat(&Token::Star) {
                    if self.at(&Token::RightParen) || self.at(&Token::Comma) {
                        // bare * means keyword-only args follow
                        continue;
                    }
                    let name = self.expect_name()?;
                    let annotation = if self.eat(&Token::Colon) {
                        Some(Box::new(self.parse_expr()?))
                    } else { None };
                    args.push(Arg { arg: name, annotation, is_vararg: true, is_kwarg: false, default: None });
                    if !self.eat(&Token::Comma) { break; }
                } else {
                    let arg = self.parse_arg()?;
                    args.push(arg);
                    if !self.eat(&Token::Comma) { break; }
                }
            }
        }
        Ok(args)
    }

    fn parse_arg(&mut self) -> Result<Arg, String> {
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
        Ok(Arg { arg, annotation, is_vararg: false, is_kwarg: false, default })
    }

    fn parse_block(&mut self) -> Result<Vec<Stmt>, String> {
        let mut stmts = Vec::new();
        if !self.eat(&Token::Newline) {
            // Single-statement body (same line after colon)
            if !self.at(&Token::Dedent) && !self.at(&Token::EndOfFile) {
                stmts.push(self.parse_stmt()?);
            }
            return Ok(stmts);
        }
        if self.eat(&Token::Indent) {
            loop {
                match &self.current {
                    Token::Dedent => {
                        self.next();
                        return Ok(stmts);
                    }
                    Token::EndOfFile => {
                        return Ok(stmts);
                    }
                    _ => {}
                }
                stmts.push(self.parse_stmt()?);
            }
        }
        Ok(stmts)
    }

    // ---- Expressions ----

    fn parse_expr(&mut self) -> Result<Expr, String> {
        self.parse_if_expr()
    }

    fn parse_if_expr(&mut self) -> Result<Expr, String> {
        let expr = self.parse_or_expr()?;
        if self.eat(&Token::If) {
            let test = self.parse_expr()?;
            self.expect(&Token::Else)?;
            let orelse = self.parse_expr()?;
            Ok(Expr::IfExp {
                test: Box::new(test),
                body: Box::new(expr),
                orelse: Box::new(orelse),
            })
        } else {
            Ok(expr)
        }
    }

    fn parse_or_expr(&mut self) -> Result<Expr, String> {
        let mut expr = self.parse_and_expr()?;
        while self.eat(&Token::Or) {
            let right = self.parse_and_expr()?;
            expr = Expr::BoolOp {
                op: BoolOp::Or,
                values: vec![expr, right],
            };
        }
        Ok(expr)
    }

    fn parse_and_expr(&mut self) -> Result<Expr, String> {
        let mut expr = self.parse_not_expr()?;
        while self.eat(&Token::And) {
            let right = self.parse_not_expr()?;
            expr = Expr::BoolOp {
                op: BoolOp::And,
                values: vec![expr, right],
            };
        }
        Ok(expr)
    }

    fn parse_not_expr(&mut self) -> Result<Expr, String> {
        if self.eat(&Token::Not) {
            let expr = self.parse_not_expr()?;
            Ok(Expr::UnaryOp {
                op: UnaryOp::Not,
                operand: Box::new(expr),
            })
        } else {
            self.parse_comparison()
        }
    }

    fn parse_comparison(&mut self) -> Result<Expr, String> {
        let mut expr = self.parse_bitwise_or()?;
        if self.at(&Token::Less) || self.at(&Token::Greater) || self.at(&Token::LessEqual)
            || self.at(&Token::GreaterEqual) || self.at(&Token::EqualEqual)
            || self.at(&Token::NotEqual) || self.at(&Token::Is)
            || self.at(&Token::In) || self.at(&Token::Not)
        {
            let mut ops = Vec::new();
            let mut comparators = Vec::new();
            loop {
                let cmp_token = self.current.clone();
                let op = match cmp_token {
                    Token::Less => { self.next(); CmpOp::Lt }
                    Token::Greater => { self.next(); CmpOp::Gt }
                    Token::LessEqual => { self.next(); CmpOp::LtE }
                    Token::GreaterEqual => { self.next(); CmpOp::GtE }
                    Token::EqualEqual => { self.next(); CmpOp::Eq }
                    Token::NotEqual => { self.next(); CmpOp::NotEq }
                    Token::Is => {
                        self.next();
                        if self.eat(&Token::Not) { CmpOp::IsNot }
                        else { CmpOp::Is }
                    }
                    Token::In => {
                        self.next();
                        CmpOp::In
                    }
                    Token::Not => {
                        if self.peek() == &Token::In {
                            self.next();
                            self.next();
                            CmpOp::NotIn
                        } else {
                            break;
                        }
                    }
                    _ => break,
                };
                ops.push(op);
                comparators.push(self.parse_bitwise_or()?);
            }
            expr = Expr::Compare {
                left: Box::new(expr),
                ops,
                comparators,
            };
        }
        Ok(expr)
    }

    fn parse_bitwise_or(&mut self) -> Result<Expr, String> {
        let mut expr = self.parse_bitwise_xor()?;
        while self.eat(&Token::Pipe) {
            let right = self.parse_bitwise_xor()?;
            expr = Expr::BinOp {
                left: Box::new(expr),
                op: Operator::BitOr,
                right: Box::new(right),
            };
        }
        Ok(expr)
    }

    fn parse_bitwise_xor(&mut self) -> Result<Expr, String> {
        let mut expr = self.parse_bitwise_and()?;
        while self.eat(&Token::Caret) {
            let right = self.parse_bitwise_and()?;
            expr = Expr::BinOp {
                left: Box::new(expr),
                op: Operator::BitXor,
                right: Box::new(right),
            };
        }
        Ok(expr)
    }

    fn parse_bitwise_and(&mut self) -> Result<Expr, String> {
        let mut expr = self.parse_shift()?;
        while self.eat(&Token::Ampersand) {
            let right = self.parse_shift()?;
            expr = Expr::BinOp {
                left: Box::new(expr),
                op: Operator::BitAnd,
                right: Box::new(right),
            };
        }
        Ok(expr)
    }

    fn parse_shift(&mut self) -> Result<Expr, String> {
        let mut expr = self.parse_term()?;
        loop {
            if self.eat(&Token::LeftShift) {
                let right = self.parse_term()?;
                expr = Expr::BinOp {
                    left: Box::new(expr),
                    op: Operator::LShift,
                    right: Box::new(right),
                };
            } else if self.eat(&Token::RightShift) {
                let right = self.parse_term()?;
                expr = Expr::BinOp {
                    left: Box::new(expr),
                    op: Operator::RShift,
                    right: Box::new(right),
                };
            } else {
                break;
            }
        }
        Ok(expr)
    }

    fn parse_term(&mut self) -> Result<Expr, String> {
        let mut expr = self.parse_factor()?;
        loop {
            if self.eat(&Token::Plus) {
                let right = self.parse_factor()?;
                expr = Expr::BinOp {
                    left: Box::new(expr),
                    op: Operator::Add,
                    right: Box::new(right),
                };
            } else if self.eat(&Token::Minus) {
                let right = self.parse_factor()?;
                expr = Expr::BinOp {
                    left: Box::new(expr),
                    op: Operator::Sub,
                    right: Box::new(right),
                };
            } else if self.eat(&Token::Star) {
                let right = self.parse_factor()?;
                expr = Expr::BinOp {
                    left: Box::new(expr),
                    op: Operator::Mult,
                    right: Box::new(right),
                };
            } else if self.eat(&Token::Slash) {
                let right = self.parse_factor()?;
                expr = Expr::BinOp {
                    left: Box::new(expr),
                    op: Operator::Div,
                    right: Box::new(right),
                };
            } else if self.eat(&Token::DoubleSlash) {
                let right = self.parse_factor()?;
                expr = Expr::BinOp {
                    left: Box::new(expr),
                    op: Operator::FloorDiv,
                    right: Box::new(right),
                };
            } else if self.eat(&Token::Percent) {
                let right = self.parse_factor()?;
                expr = Expr::BinOp {
                    left: Box::new(expr),
                    op: Operator::Mod,
                    right: Box::new(right),
                };
            } else if self.eat(&Token::At) {
                let right = self.parse_factor()?;
                expr = Expr::BinOp {
                    left: Box::new(expr),
                    op: Operator::MatMult,
                    right: Box::new(right),
                };
            } else {
                break;
            }
        }
        Ok(expr)
    }

    fn parse_factor(&mut self) -> Result<Expr, String> {
        self.parse_unary()
    }

    fn parse_unary(&mut self) -> Result<Expr, String> {
        if self.eat(&Token::Plus) {
            let expr = self.parse_unary()?;
            Ok(Expr::UnaryOp {
                op: UnaryOp::UAdd,
                operand: Box::new(expr),
            })
        } else if self.eat(&Token::Minus) {
            let expr = self.parse_unary()?;
            Ok(Expr::UnaryOp {
                op: UnaryOp::USub,
                operand: Box::new(expr),
            })
        } else if self.eat(&Token::Tilde) {
            let expr = self.parse_unary()?;
            Ok(Expr::UnaryOp {
                op: UnaryOp::Invert,
                operand: Box::new(expr),
            })
        } else {
            self.parse_power()
        }
    }

    fn parse_power(&mut self) -> Result<Expr, String> {
        let mut expr = self.parse_primary()?;
        if self.eat(&Token::DoubleStar) {
            let right = self.parse_unary()?;
            expr = Expr::BinOp {
                left: Box::new(expr),
                op: Operator::Pow,
                right: Box::new(right),
            };
        }
        Ok(expr)
    }

    fn parse_primary(&mut self) -> Result<Expr, String> {
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
                if !self.at(&Token::RightParen) {
                    loop {
                        if self.at(&Token::Star) {
                            let starred = self.parse_expr()?;
                            args.push(Expr::Starred(Box::new(starred)));
                        } else if self.at(&Token::DoubleStar) {
                            self.next();
                            let value = self.parse_expr()?;
                            keywords.push(Keyword { arg: None, value: Box::new(value) });
                        } else if self.peek() == &Token::Equal && matches!(&self.current, Token::Name(_)) {
                            let arg = Some(self.expect_name()?);
                            self.expect(&Token::Equal)?;
                            let value = self.parse_expr()?;
                            keywords.push(Keyword { arg, value: Box::new(value) });
                        } else {
                            args.push(self.parse_expr()?);
                        }
                        if !self.eat(&Token::Comma) { break; }
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

    fn parse_slice_or_expr(&mut self) -> Result<Expr, String> {
        if self.eat(&Token::Colon) {
            let mut upper = None;
            let mut step = None;
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
            return Ok(Expr::Slice { lower: None, upper, step });
        }
        let lower = self.parse_expr()?;
        if self.eat(&Token::Colon) {
            let upper = if !self.at(&Token::RightBracket) && !self.at(&Token::Comma) {
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
            Ok(Expr::Slice {
                lower: Some(Box::new(lower)),
                upper,
                step,
            })
        } else {
            Ok(lower)
        }
    }

    fn parse_atom(&mut self) -> Result<Expr, String> {
        match &self.current {
            Token::None => { self.next(); Ok(Expr::Constant(Constant::None)) }
            Token::True => { self.next(); Ok(Expr::Constant(Constant::Bool(true))) }
            Token::False => { self.next(); Ok(Expr::Constant(Constant::Bool(false))) }
            Token::Ellipsis => { self.next(); Ok(Expr::Constant(Constant::Ellipsis)) }

            Token::Number(s) => {
                let s = s.clone();
                self.next();
                if s.contains('.') || s.contains('e') || s.contains('E') {
                    Ok(Expr::Constant(Constant::float_from_str(&s)))
                } else if s.ends_with('j') || s.ends_with('J') {
                    let real = s[..s.len()-1].to_string();
                    Ok(Expr::Constant(Constant::Complex { real: "0".to_string(), imag: real }))
                } else {
                    Ok(Expr::Constant(Constant::int_from_str(&s)))
                }
            }

            Token::String(s) => {
                let s = s.clone();
                self.next();
                let mut parts = vec![Expr::Constant(Constant::String(s))];
                while let Token::String(s2) = &self.current {
                    parts.push(Expr::Constant(Constant::String(s2.clone())));
                    self.next();
                }
                if parts.len() == 1 {
                    Ok(parts.into_iter().next().unwrap())
                } else {
                    Ok(Expr::JoinedStr(parts))
                }
            }

            Token::Bytes(b) => {
                let b = b.clone();
                self.next();
                Ok(Expr::Constant(Constant::Bytes(b)))
            }

            Token::FStringStart => {
                self.parse_fstring()
            }

            Token::Name(s) => {
                let name = s.clone();
                self.next();
                Ok(Expr::Name(name))
            }

            Token::LeftParen => {
                self.next();
                let expr = if self.eat(&Token::RightParen) {
                    Expr::Tuple(Vec::new()) // empty tuple
                } else if self.peek() == &Token::Comma || (self.peek() == &Token::Equal && matches!(&self.current, Token::Name(_))) {
                    // Single-element tuple or named expression
                    let first = self.parse_expr()?;
                    if self.eat(&Token::Comma) {
                        let mut elts = vec![first];
                        while !self.at(&Token::RightParen) && !self.at(&Token::EndOfFile) {
                            elts.push(self.parse_expr()?);
                            if !self.eat(&Token::Comma) { break; }
                        }
                        self.expect(&Token::RightParen)?;
                        Expr::Tuple(elts)
                    } else if self.eat(&Token::Walrus) {
                        // Walrus operator (:=)
                        let value = self.parse_expr()?;
                        self.expect(&Token::RightParen)?;
                        Expr::NamedExpr {
                            target: Box::new(first),
                            value: Box::new(value),
                        }
                    } else {
                        self.expect(&Token::RightParen)?;
                        first
                    }
                } else {
                    let first = self.parse_bitwise_or()?;
                    if self.eat(&Token::For) {
                        // Generator expression: (expr for x in iter)
                        let target = self.parse_bitwise_or()?;
                        self.expect(&Token::In)?;
                        let iter = self.parse_or_expr()?;
                        let mut generators = vec![Comprehension { target: Box::new(target), iter: Box::new(iter), ifs: Vec::new(), is_async: false }];
                        while self.eat(&Token::For) {
                            let t = self.parse_bitwise_or()?;
                            self.expect(&Token::In)?;
                            let i = self.parse_or_expr()?;
                            generators.push(Comprehension { target: Box::new(t), iter: Box::new(i), ifs: Vec::new(), is_async: false });
                        }
                        if self.eat(&Token::If) {
                            if let Some(last) = generators.last_mut() {
                                last.ifs.push(self.parse_or_expr()?);
                                while self.eat(&Token::If) {
                                    last.ifs.push(self.parse_or_expr()?);
                                }
                            }
                        }
                        self.expect(&Token::RightParen)?;
                        Expr::GeneratorExp { elt: Box::new(first), generators }
                    } else if self.eat(&Token::Walrus) {
                        // Walrus operator: (x := expr)
                        let value = self.parse_expr()?;
                        self.expect(&Token::RightParen)?;
                        Expr::NamedExpr {
                            target: Box::new(first),
                            value: Box::new(value),
                        }
                    } else if self.eat(&Token::Comma) {
                        let mut elts = vec![first];
                        loop {
                            if self.at(&Token::RightParen) { break; }
                            elts.push(self.parse_expr()?);
                            if !self.eat(&Token::Comma) { break; }
                        }
                        self.expect(&Token::RightParen)?;
                        Expr::Tuple(elts)
                    } else {
                        self.expect(&Token::RightParen)?;
                        first
                    }
                };
                Ok(expr)
            }

            Token::LeftBracket => {
                self.next();
                let mut elts = Vec::new();
                if !self.at(&Token::RightBracket) {
                    loop {
                        if self.eat(&Token::DoubleStar) {
                            let expr = self.parse_expr()?;
                            elts.push(Expr::Starred(Box::new(expr)));
                        } else {
                            elts.push(self.parse_bitwise_or()?);
                        }
                        if !self.eat(&Token::Comma) { break; }
                    }
                }
                // Check for list comprehension: [expr for ...]
                if elts.len() == 1 && self.eat(&Token::For) {
                    let target = self.parse_bitwise_or()?;
                    self.expect(&Token::In)?;
                    let iter = self.parse_or_expr()?;
                    let mut generators = vec![Comprehension { target: Box::new(target), iter: Box::new(iter), ifs: Vec::new(), is_async: false }];
                    while self.eat(&Token::For) {
                        let t = self.parse_bitwise_or()?;
                        self.expect(&Token::In)?;
                        let i = self.parse_or_expr()?;
                        generators.push(Comprehension { target: Box::new(t), iter: Box::new(i), ifs: Vec::new(), is_async: false });
                    }
                    // Optional if clauses
                    if self.eat(&Token::If) {
                        if let Some(last) = generators.last_mut() {
                            last.ifs.push(self.parse_or_expr()?);
                            while self.eat(&Token::If) {
                                last.ifs.push(self.parse_or_expr()?);
                            }
                        }
                    }
                    self.expect(&Token::RightBracket)?;
                    return Ok(Expr::ListComp {
                        elt: Box::new(elts.into_iter().next().unwrap()),
                        generators,
                    });
                }
                self.expect(&Token::RightBracket)?;
                Ok(Expr::List(elts))
            }

            Token::LeftBrace => {
                self.next();
                let mut keys = Vec::new();
                let mut values = Vec::new();
                let mut is_dict = false;
                if !self.at(&Token::RightBrace) {
                    // Parse first element to check for comprehension
                    if self.eat(&Token::DoubleStar) {
                        let expr = self.parse_expr()?;
                        keys.push(None);
                        values.push(expr);
                        is_dict = true;
                    } else {
                        let key = self.parse_expr()?;
                        if self.eat(&Token::Colon) {
                            let value = self.parse_expr()?;
                            // Check for dict comprehension: {k: v for ...}
                            if self.eat(&Token::For) {
                                let target = self.parse_bitwise_or()?;
                                self.expect(&Token::In)?;
                                let iter = self.parse_or_expr()?;
                                let mut generators = vec![Comprehension { target: Box::new(target), iter: Box::new(iter), ifs: Vec::new(), is_async: false }];
                                while self.eat(&Token::For) {
                                    let t = self.parse_bitwise_or()?;
                                    self.expect(&Token::In)?;
                                    let i = self.parse_or_expr()?;
                                    generators.push(Comprehension { target: Box::new(t), iter: Box::new(i), ifs: Vec::new(), is_async: false });
                                }
                                if self.eat(&Token::If) {
                                    if let Some(last) = generators.last_mut() {
                                        last.ifs.push(self.parse_or_expr()?);
                                        while self.eat(&Token::If) {
                                            last.ifs.push(self.parse_or_expr()?);
                                        }
                                    }
                                }
                                self.expect(&Token::RightBrace)?;
                                return Ok(Expr::DictComp {
                                    key: Box::new(key),
                                    value: Box::new(value),
                                    generators,
                                });
                            }
                            keys.push(Some(key));
                            values.push(value);
                            is_dict = true;
                        } else {
                            // Check for set comprehension: {expr for ...}
                            if self.eat(&Token::For) {
                                let target = self.parse_bitwise_or()?;
                                self.expect(&Token::In)?;
                                let iter = self.parse_or_expr()?;
                                let mut generators = vec![Comprehension { target: Box::new(target), iter: Box::new(iter), ifs: Vec::new(), is_async: false }];
                                while self.eat(&Token::For) {
                                    let t = self.parse_bitwise_or()?;
                                    self.expect(&Token::In)?;
                                    let i = self.parse_or_expr()?;
                                    generators.push(Comprehension { target: Box::new(t), iter: Box::new(i), ifs: Vec::new(), is_async: false });
                                }
                                if self.eat(&Token::If) {
                                    if let Some(last) = generators.last_mut() {
                                        last.ifs.push(self.parse_or_expr()?);
                                        while self.eat(&Token::If) {
                                            last.ifs.push(self.parse_or_expr()?);
                                        }
                                    }
                                }
                                self.expect(&Token::RightBrace)?;
                                return Ok(Expr::SetComp {
                                    elt: Box::new(key),
                                    generators,
                                });
                            }
                            values.push(key);
                        }
                    }
                    // Parse remaining elements
                    while self.eat(&Token::Comma) {
                        if self.at(&Token::RightBrace) { break; }
                        if self.eat(&Token::DoubleStar) {
                            let expr = self.parse_expr()?;
                            keys.push(None);
                            values.push(expr);
                            is_dict = true;
                        } else {
                            let k = self.parse_expr()?;
                            if self.eat(&Token::Colon) {
                                let v = self.parse_expr()?;
                                keys.push(Some(k));
                                values.push(v);
                                is_dict = true;
                            } else {
                                values.push(k);
                            }
                        }
                    }
                }
                self.expect(&Token::RightBrace)?;
                if is_dict || values.is_empty() {
                    Ok(Expr::Dict { keys, values })
                } else {
                    Ok(Expr::Set(values))
                }
            }

            Token::Lambda => {
                self.parse_lambda()
            }

            Token::Yield => {
                self.parse_yield_expr()
            }

            Token::Await => {
                self.next();
                let expr = self.parse_unary()?;
                Ok(Expr::Await(Box::new(expr)))
            }

            _ => unexpected_token!(self, "expression"),
        }
    }

    fn parse_fstring(&mut self) -> Result<Expr, String> {
        let mut parts = Vec::new();
        loop {
            match &self.current {
                Token::FStringMiddle(s) => {
                    let s = s.clone();
                    self.next();
                    parts.push(FStringPart::String(s));
                }
                Token::FStringStart => {
                    self.next();
                }
                Token::FStringEnd => {
                    self.next();
                    break;
                }
                Token::LeftBrace => {
                    unreachable!("fstring: should not see {{");
                }
                _ => {
                    if self.at(&Token::EndOfFile) {
                        break;
                    }
                    let expr = self.parse_expr()?;
                    parts.push(FStringPart::Expr(Box::new(expr)));
                }
            }
        }
        Ok(Expr::FString(parts))
    }

    fn parse_lambda(&mut self) -> Result<Expr, String> {
        self.next();
        let args = if self.eat(&Token::Colon) {
            Vec::new()
        } else {
            let args = self.parse_lambda_args()?;
            self.expect(&Token::Colon)?;
            args
        };
        let body = self.parse_expr()?;
        Ok(Expr::Lambda {
            args,
            body: Box::new(body),
        })
    }

    fn parse_lambda_args(&mut self) -> Result<Vec<Arg>, String> {
        let mut args = Vec::new();
        loop {
            if self.at(&Token::Colon) { break; }
            if self.eat(&Token::Star) {
                if self.at(&Token::Colon) || self.at(&Token::Comma) {
                    continue;
                }
                let name = self.expect_name()?;
                args.push(Arg { arg: name, annotation: None, is_vararg: true, is_kwarg: false, default: None });
            } else if self.eat(&Token::DoubleStar) {
                let name = self.expect_name()?;
                args.push(Arg { arg: name, annotation: None, is_vararg: false, is_kwarg: true, default: None });
            } else {
                let name = self.expect_name()?;
                args.push(Arg { arg: name, annotation: None, is_vararg: false, is_kwarg: false, default: None });
            }
            if !self.eat(&Token::Comma) { break; }
        }
        Ok(args)
    }

    fn parse_yield_expr(&mut self) -> Result<Expr, String> {
        self.next();
        if self.eat(&Token::From) {
            let expr = self.parse_expr()?;
            Ok(Expr::YieldFrom(Box::new(expr)))
        } else {
            let expr = if !self.at(&Token::Newline) && !self.at(&Token::RightParen)
                && !self.at(&Token::RightBracket) && !self.at(&Token::RightBrace)
                && !self.at(&Token::Colon) && !self.at(&Token::Comma)
                && !self.at(&Token::Semicolon) && !self.at(&Token::EndOfFile)
            {
                Some(Box::new(self.parse_expr()?))
            } else {
                None
            };
            Ok(Expr::Yield(expr))
        }
    }
}
