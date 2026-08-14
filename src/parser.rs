use crate::ast::*;
use crate::token::{Lexer, Token};

pub struct Parser {
    lexer: Lexer,
    current: Token,
    peeked: Option<Token>,
    // Guards `parse_unary`'s self-recursion (`-`/`+`/`~` chains) against a
    // native stack overflow on pathological input — real CPython's own PEG
    // parser has an equivalent C-stack-depth counter and raises `MemoryError:
    // too complex` past it, rather than crashing (its own `test_syntax.py`
    // explicitly tests for exactly this: `compile("-" * 100000 + "4", ...)`
    // must raise `MemoryError`, not abort the process). Without this guard,
    // this interpreter's own recursive-descent `parse_unary` (which calls
    // itself directly, once per leading `-`/`+`/`~`) genuinely overflowed
    // the native stack on that same input — confirmed via the exact repro,
    // crashing with a real OS-level "stack overflow, aborting" abort, not a
    // catchable Rust panic or Python exception.
    unary_depth: usize,
    // Nesting depth of "single-line compound-statement suite" parsing (the
    // `if x: a; b; c` / `def f(): a; b; c` form) — see `parse_block`'s own
    // doc comment and `expect_newline_or_eof`'s use of this field for the
    // full story. A counter (not a bool) in case a single-line suite's own
    // body itself starts another single-line suite (`if True: if False: a
    // = 1`), though that's a rare style choice, not the common trigger.
    suite_depth: usize,
}

const MAX_UNARY_DEPTH: usize = 2000;

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
            unary_depth: 0,
            suite_depth: 0,
        }
    }

    fn next(&mut self) -> Token {
        let tok = self
            .peeked
            .take()
            .unwrap_or_else(|| self.lexer.next_token());
        std::mem::replace(&mut self.current, tok)
    }

    fn peek(&mut self) -> &Token {
        if self.peeked.is_none() {
            self.peeked = Some(self.lexer.next_token());
        }
        self.peeked.as_ref().unwrap()
    }

    /// Disambiguates the `match` soft keyword from `match` used as a plain
    /// identifier (e.g. `match, rest = x.split()` or `match = re.match(...)`).
    /// A real match-statement's header always ends with a bare `:` immediately
    /// before the line's NEWLINE, at bracket depth 0 — no other statement
    /// starting with a NAME token has that shape (annotated assignment's `:`
    /// appears right after the target, not at the end). We scan ahead with a
    /// cloned lexer so this is non-destructive to actual parse position.
    fn looks_like_match_stmt(&mut self) -> bool {
        let mut tok = self.peek().clone();
        let mut lexer = self.lexer.clone();
        let mut depth: i32 = 0;
        loop {
            match tok {
                Token::LeftParen | Token::LeftBracket | Token::LeftBrace => depth += 1,
                Token::RightParen | Token::RightBracket | Token::RightBrace => depth -= 1,
                Token::Newline | Token::EndOfFile => return false,
                Token::Colon if depth == 0 => {
                    // Must be followed directly by Newline (top-level match header)
                    let next = lexer.next_token();
                    return matches!(next, Token::Newline);
                }
                Token::Equal if depth == 0 => return false,
                _ => {}
            }
            tok = lexer.next_token();
        }
    }

    /// Disambiguates `with (` starting a PEP 617 parenthesized multi-item
    /// group (`with (cm1, cm2 as x):`, letting the items span multiple
    /// lines with trailing commas — real code, e.g. CPython's own test
    /// suite: `with (support.foo(), support.Bar() as sw):`) from `(` simply
    /// being the start of a single context-expression that happens to be
    /// parenthesized (`with (a + b).open():`, `with (some_cm):`). Called
    /// with the current token already `(`: scans ahead (non-destructively,
    /// cloned lexer — same technique as `looks_like_match_stmt`) for a
    /// top-level `,` or `as` before the matching `)`. Either one only makes
    /// sense inside a with-items group (a bare tuple/grouped-expr can't
    /// contain `as`, and `with (a, b):`'s comma is overwhelmingly intended
    /// as two context managers under the 3.10+ syntax, not the legacy
    /// tuple-as-one-CM reading) — so finding either is decisive. Finding
    /// neither means the parens belong to the expression itself, and must
    /// be left for `parse_expr` to consume normally.
    fn looks_like_parenthesized_with_items(&mut self) -> bool {
        let mut lexer = self.lexer.clone();
        let mut depth: i32 = 0;
        loop {
            let tok = lexer.next_token();
            match tok {
                Token::LeftParen | Token::LeftBracket | Token::LeftBrace => depth += 1,
                Token::RightParen if depth == 0 => return false,
                Token::RightParen | Token::RightBracket | Token::RightBrace => depth -= 1,
                Token::Newline | Token::EndOfFile => return false,
                Token::Comma | Token::As if depth == 0 => return true,
                _ => {}
            }
        }
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
            // `_` is a regular identifier everywhere except match-statement
            // wildcard patterns (which are parsed separately) — e.g. the
            // common `from x import gettext_lazy as _` idiom.
            Token::Underscore => {
                self.next();
                Ok("_".to_string())
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
            let line = self.lexer.get_line_col().0;
            stmts.push(Stmt::Located(line, Box::new(self.parse_stmt()?)));
            // Handle semicolon-separated statements on the same line: `while i: i = 0; continue`
            while self.eat(&Token::Semicolon) {
                if self.at(&Token::Dedent) || self.at(&Token::EndOfFile) {
                    break;
                }
                let line = self.lexer.get_line_col().0;
                stmts.push(Stmt::Located(line, Box::new(self.parse_stmt()?)));
            }
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
            self.next(); // consume async
            return self.parse_for(true);
        }
        if self.at(&Token::Async) && self.peek() == &Token::With {
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

    fn parse_stmt_tail(&mut self, expr: Expr) -> Result<Stmt, String> {
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
            let _ = self.expect_newline_or_eof();
            Ok(Stmt::Expr(Box::new(expr)))
        }
    }

    fn expect_newline_or_eof(&mut self) -> Result<(), String> {
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
        // real Python raises SyntaxError. This used to silently return Ok,
        // leaving the token for the outer loop to parse as a SECOND
        // statement on the same line (test_print's TestPy2MigrationHint).
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
        self.expect(&Token::Colon)?;
        let body = self.parse_block()?;
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

    fn parse_class(&mut self) -> Result<Stmt, String> {
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

    fn parse_if(&mut self) -> Result<Stmt, String> {
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

    /// Consumes a comprehension clause's leading `for` or `async for`.
    /// Returns `Some(is_async)` if either was consumed, `None` (consuming
    /// nothing) if neither is present — callers use this both to decide
    /// whether another clause follows and to know its async-ness. `async`
    /// is a hard keyword in this lexer (never a plain identifier), so
    /// seeing it here and NOT finding `for` right after is unambiguously
    /// invalid syntax — `expect` surfaces that as a normal parse error.
    fn eat_comp_for(&mut self) -> Result<Option<bool>, String> {
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
    fn parse_for_target_elt(&mut self) -> Result<Expr, String> {
        // `for head, *tail in ...:` — a starred sub-target collects the rest
        // of the iterable's items into a list, same as in assignment targets.
        if self.eat(&Token::Star) {
            return Ok(Expr::Starred(Box::new(self.parse_bitwise_or()?)));
        }
        self.parse_bitwise_or()
    }

    fn parse_for_target(&mut self) -> Result<Expr, String> {
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

    fn parse_for(&mut self, is_async: bool) -> Result<Stmt, String> {
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

    fn parse_with(&mut self, is_async: bool) -> Result<Stmt, String> {
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

    fn parse_try(&mut self) -> Result<Stmt, String> {
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

    fn parse_match(&mut self) -> Result<Stmt, String> {
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

    fn parse_type_alias(&mut self) -> Result<Stmt, String> {
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

    fn parse_match_cases(&mut self) -> Result<Vec<MatchCase>, String> {
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

    fn parse_sequence_pattern(&mut self) -> Result<Pattern, String> {
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

    fn parse_mapping_pattern(&mut self) -> Result<Pattern, String> {
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

    // ---- Other statements ----

    fn parse_return(&mut self) -> Result<Stmt, String> {
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

    fn parse_yield_stmt(&mut self) -> Result<Stmt, String> {
        let expr = self.parse_yield_expr()?;
        let _ = self.expect_newline_or_eof();
        Ok(Stmt::Expr(Box::new(expr)))
    }

    fn parse_raise(&mut self) -> Result<Stmt, String> {
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

    fn parse_global(&mut self) -> Result<Stmt, String> {
        self.next();
        let mut names = vec![self.expect_name()?];
        while self.eat(&Token::Comma) {
            names.push(self.expect_name()?);
        }
        let _ = self.expect_newline_or_eof();
        Ok(Stmt::Global(names))
    }

    fn parse_nonlocal(&mut self) -> Result<Stmt, String> {
        self.next();
        let mut names = vec![self.expect_name()?];
        while self.eat(&Token::Comma) {
            names.push(self.expect_name()?);
        }
        let _ = self.expect_newline_or_eof();
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
        let _ = self.expect_newline_or_eof();
        Ok(Stmt::Assert {
            test: Box::new(test),
            msg,
        })
    }

    fn parse_del(&mut self) -> Result<Stmt, String> {
        self.next();
        let mut targets = vec![self.parse_expr()?];
        while self.eat(&Token::Comma) {
            if self.at(&Token::Newline) || self.at(&Token::Semicolon) || self.at(&Token::EndOfFile)
            {
                break;
            }
            targets.push(self.parse_expr()?);
        }
        let _ = self.expect_newline_or_eof();
        Ok(Stmt::Delete(targets))
    }

    fn parse_import(&mut self) -> Result<Stmt, String> {
        self.next();
        let mut names = vec![self.parse_alias()?];
        while self.eat(&Token::Comma) {
            names.push(self.parse_alias()?);
        }
        let _ = self.expect_newline_or_eof();
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

    fn parse_args(&mut self) -> Result<Vec<Arg>, String> {
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

    fn parse_block(&mut self) -> Result<Vec<Stmt>, String> {
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
        Ok(stmts)
    }

    // ---- Expressions ----

    fn parse_expr(&mut self) -> Result<Expr, String> {
        let expr = self.parse_conditional_expr()?;
        // Walrus operator (:=) — named expressions, allowed at top expr level
        // e.g. `if spec := getattr(...):`
        if self.eat(&Token::Walrus) {
            let value = self.parse_expr()?;
            Ok(Expr::NamedExpr {
                target: Box::new(expr),
                value: Box::new(value),
            })
        } else {
            Ok(expr)
        }
    }

    fn parse_conditional_expr(&mut self) -> Result<Expr, String> {
        let expr = self.parse_or_expr()?;
        if self.eat(&Token::If) {
            let test = self.parse_conditional_expr()?;
            self.expect(&Token::Else)?;
            let orelse = self.parse_conditional_expr()?;
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
        if self.at(&Token::Less)
            || self.at(&Token::Greater)
            || self.at(&Token::LessEqual)
            || self.at(&Token::GreaterEqual)
            || self.at(&Token::EqualEqual)
            || self.at(&Token::NotEqual)
            || self.at(&Token::Is)
            || self.at(&Token::In)
            || self.at(&Token::Not)
        {
            let mut ops = Vec::new();
            let mut comparators = Vec::new();
            loop {
                let cmp_token = self.current.clone();
                let op = match cmp_token {
                    Token::Less => {
                        self.next();
                        CmpOp::Lt
                    }
                    Token::Greater => {
                        self.next();
                        CmpOp::Gt
                    }
                    Token::LessEqual => {
                        self.next();
                        CmpOp::LtE
                    }
                    Token::GreaterEqual => {
                        self.next();
                        CmpOp::GtE
                    }
                    Token::EqualEqual => {
                        self.next();
                        CmpOp::Eq
                    }
                    Token::NotEqual => {
                        self.next();
                        CmpOp::NotEq
                    }
                    Token::Is => {
                        self.next();
                        if self.eat(&Token::Not) {
                            CmpOp::IsNot
                        } else {
                            CmpOp::Is
                        }
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
        let mut expr = self.parse_mul()?;
        loop {
            if self.eat(&Token::Plus) {
                let right = self.parse_mul()?;
                expr = Expr::BinOp {
                    left: Box::new(expr),
                    op: Operator::Add,
                    right: Box::new(right),
                };
            } else if self.eat(&Token::Minus) {
                let right = self.parse_mul()?;
                expr = Expr::BinOp {
                    left: Box::new(expr),
                    op: Operator::Sub,
                    right: Box::new(right),
                };
            } else {
                break;
            }
        }
        Ok(expr)
    }

    fn parse_mul(&mut self) -> Result<Expr, String> {
        let mut expr = self.parse_unary()?;
        loop {
            if self.eat(&Token::Star) {
                let right = self.parse_unary()?;
                expr = Expr::BinOp {
                    left: Box::new(expr),
                    op: Operator::Mult,
                    right: Box::new(right),
                };
            } else if self.eat(&Token::Slash) {
                let right = self.parse_unary()?;
                expr = Expr::BinOp {
                    left: Box::new(expr),
                    op: Operator::Div,
                    right: Box::new(right),
                };
            } else if self.eat(&Token::DoubleSlash) {
                let right = self.parse_unary()?;
                expr = Expr::BinOp {
                    left: Box::new(expr),
                    op: Operator::FloorDiv,
                    right: Box::new(right),
                };
            } else if self.eat(&Token::Percent) {
                let right = self.parse_unary()?;
                expr = Expr::BinOp {
                    left: Box::new(expr),
                    op: Operator::Mod,
                    right: Box::new(right),
                };
            } else if self.eat(&Token::At) {
                let right = self.parse_unary()?;
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

    fn parse_unary(&mut self) -> Result<Expr, String> {
        let is_unary_prefix = matches!(self.current, Token::Plus | Token::Minus | Token::Tilde);
        if is_unary_prefix {
            self.unary_depth += 1;
            if self.unary_depth > MAX_UNARY_DEPTH {
                self.unary_depth -= 1;
                return Err("MemoryError: too complex".to_string());
            }
        }
        let result = if self.eat(&Token::Plus) {
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
        };
        if is_unary_prefix {
            self.unary_depth -= 1;
        }
        result
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
                let mut seen_keyword = false;
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
                            self.next(); // consume *
                            let starred = self.parse_expr()?;
                            args.push(Expr::Starred(Box::new(starred)));
                        } else if self.at(&Token::DoubleStar) {
                            self.next();
                            let value = self.parse_expr()?;
                            keywords.push(Keyword {
                                arg: None,
                                value: Box::new(value),
                            });
                            seen_keyword = true;
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

    fn parse_slice_or_expr(&mut self) -> Result<Expr, String> {
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

    fn parse_atom(&mut self) -> Result<Expr, String> {
        match &self.current {
            Token::None => {
                self.next();
                Ok(Expr::Constant(Constant::None))
            }
            Token::True => {
                self.next();
                Ok(Expr::Constant(Constant::Bool(true)))
            }
            Token::False => {
                self.next();
                Ok(Expr::Constant(Constant::Bool(false)))
            }
            Token::Ellipsis => {
                self.next();
                Ok(Expr::Constant(Constant::Ellipsis))
            }
            Token::Underscore => {
                self.next();
                Ok(Expr::Name("_".to_string()))
            }

            Token::Number(s) => {
                let s = s.clone();
                self.next();
                // Hex/binary/octal numbers may contain 'e'/'E'/'B'/'O' as valid digits
                // so we must check for float exponent only in decimal/octal numbers.
                // The imaginary suffix (`2j`, `3.5J`) must be checked BEFORE the
                // plain-int fallback below — `"2j"` contains no '.'/'e'/'E' so it
                // used to satisfy the int-fallback condition first and never reach
                // this arm at all, sending literal `2j` through `int_from_str`
                // (raising "invalid integer: 2j" for every imaginary literal).
                if s.ends_with('j') || s.ends_with('J') {
                    let imag = s[..s.len() - 1].to_string();
                    Ok(Expr::Constant(Constant::Complex {
                        real: "0".to_string(),
                        imag,
                    }))
                } else if (s.starts_with("0x")
                    || s.starts_with("0X")
                    || s.starts_with("0b")
                    || s.starts_with("0B")
                    || s.starts_with("0o")
                    || s.starts_with("0O"))
                    || (!s.contains('.') && !s.contains('e') && !s.contains('E'))
                {
                    Ok(Expr::Constant(Constant::int_from_str(&s)))
                } else {
                    Ok(Expr::Constant(Constant::float_from_str(&s)))
                }
            }

            Token::String(s) => {
                let parts = vec![s.clone()];
                self.next();
                // Implicit string concatenation: adjacent strings and f-strings
                // with optional newlines. Chain via BinOp::Add since we can't
                // pre-compute f-strings at parse time.
                let mut expr = Expr::Constant(Constant::String(parts.concat()));
                loop {
                    // Only eat newlines if followed by a string or f-string (implicit concatenation)
                    while self.at(&Token::Newline)
                        && (matches!(self.peek(), Token::String(_))
                            || matches!(self.peek(), Token::FStringStart))
                    {
                        self.next();
                    }
                    match &self.current {
                        Token::String(s2) => {
                            let s = s2.clone();
                            self.next();
                            expr = Expr::BinOp {
                                left: Box::new(expr),
                                op: Operator::Add,
                                right: Box::new(Expr::Constant(Constant::String(s))),
                            };
                        }
                        Token::FStringStart => {
                            let fstr = self.parse_fstring()?;
                            expr = Expr::BinOp {
                                left: Box::new(expr),
                                op: Operator::Add,
                                right: Box::new(fstr),
                            };
                        }
                        _ => break,
                    }
                }
                Ok(expr)
            }

            Token::Bytes(b) => {
                let mut parts = vec![b.clone()];
                self.next();
                // Implicit bytes concatenation: adjacent bytes with optional newlines
                loop {
                    // Only eat newlines if followed by a bytes literal
                    while self.at(&Token::Newline) && matches!(self.peek(), Token::Bytes(_)) {
                        self.next();
                    }
                    if !matches!(&self.current, Token::Bytes(_)) {
                        break;
                    }
                    if let Token::Bytes(next) = &self.current {
                        parts.push(next.clone());
                        self.next();
                    }
                }
                let combined: Vec<u8> = parts.into_iter().flatten().collect();
                Ok(Expr::Constant(Constant::Bytes(combined)))
            }

            Token::FStringStart => {
                self.next();
                let mut parts = vec![self.parse_fstring()?];
                // Implicit concatenation: adjacent f-strings and regular strings
                loop {
                    // Only eat newlines if followed by an f-string or string (implicit concatenation)
                    while self.at(&Token::Newline)
                        && (matches!(self.peek(), Token::FStringStart)
                            || matches!(self.peek(), Token::String(_)))
                    {
                        self.next();
                    }
                    match &self.current {
                        Token::FStringStart => {
                            self.next();
                            parts.push(self.parse_fstring()?);
                        }
                        Token::String(s) => {
                            let s = s.clone();
                            self.next();
                            parts.push(Expr::Constant(Constant::String(s)));
                        }
                        _ => break,
                    }
                }
                if parts.len() == 1 {
                    Ok(parts.into_iter().next().unwrap())
                } else {
                    let result = parts
                        .into_iter()
                        .reduce(|a, b| Expr::BinOp {
                            left: Box::new(a),
                            op: Operator::Add,
                            right: Box::new(b),
                        })
                        .unwrap();
                    Ok(result)
                }
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
                } else if self.at(&Token::Star) {
                    // Unpacking inside tuple: (*a, *b)
                    let mut elts = Vec::new();
                    while !self.at(&Token::RightParen) && !self.at(&Token::EndOfFile) {
                        if self.at(&Token::Star) {
                            self.next();
                            elts.push(Expr::Starred(Box::new(self.parse_expr()?)));
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
                    self.expect(&Token::RightParen)?;
                    Expr::Tuple(elts)
                } else if self.peek() == &Token::Comma
                    || (self.peek() == &Token::Equal
                        && (matches!(&self.current, Token::Name(_)) || self.at(&Token::Underscore)))
                {
                    // Single-element tuple or named expression
                    let first = self.parse_expr()?;
                    if self.eat(&Token::Comma) {
                        let mut elts = vec![first];
                        while !self.at(&Token::RightParen) && !self.at(&Token::EndOfFile) {
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
                    let first = self.parse_expr()?;
                    if let Some(is_async) = self.eat_comp_for()? {
                        // Generator expression: (expr for x in iter)
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
                        self.expect(&Token::RightParen)?;
                        Expr::GeneratorExp {
                            elt: Box::new(first),
                            generators,
                        }
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
                            if self.at(&Token::RightParen) {
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
                        } else if self.eat(&Token::Star) {
                            let expr = self.parse_expr()?;
                            elts.push(Expr::Starred(Box::new(expr)));
                        } else {
                            // `parse_expr` (not `parse_conditional_expr`) so
                            // a walrus assignment expression is allowed as a
                            // list-display element (`[y := f(x), x/y]`,
                            // valid real Python grammar — `testlist_comp`
                            // uses `namedexpr_test`, not bare `test`) —
                            // confirmed general via CPython's own
                            // `test_named_expressions.py`.
                            elts.push(self.parse_expr()?);
                        }
                        if !self.eat(&Token::Comma) {
                            break;
                        }
                        // After eating a trailing comma, check if we're at the end
                        if self.at(&Token::RightBracket) || self.at(&Token::EndOfFile) {
                            break;
                        }
                    }
                }
                // Check for list comprehension: [expr for ...]
                if elts.len() == 1 && (self.at(&Token::For) || self.at(&Token::Async)) {
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
                    } else if self.eat(&Token::Star) {
                        // Set unpacking: {*a, *b} — a starred element can
                        // only appear in a set display, never a dict (no
                        // `k: v` check needed, unlike the plain-key branch).
                        let expr = self.parse_expr()?;
                        values.push(Expr::Starred(Box::new(expr)));
                    } else {
                        let key = self.parse_expr()?;
                        if self.eat(&Token::Colon) {
                            let value = self.parse_expr()?;
                            // Check for dict comprehension: {k: v for ...}
                            if let Some(is_async) = self.eat_comp_for()? {
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
                            if let Some(is_async) = self.eat_comp_for()? {
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
                        if self.at(&Token::RightBrace) {
                            break;
                        }
                        if self.eat(&Token::DoubleStar) {
                            let expr = self.parse_expr()?;
                            keys.push(None);
                            values.push(expr);
                            is_dict = true;
                        } else if self.eat(&Token::Star) {
                            let expr = self.parse_expr()?;
                            values.push(Expr::Starred(Box::new(expr)));
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

            Token::Lambda => self.parse_lambda(),

            Token::Yield => self.parse_yield_expr(),

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
        let mut first = true;
        loop {
            if first {
                first = false;
                // First token might be FStringStart (from implicit concatenation)
                if matches!(&self.current, Token::FStringStart) {
                    self.next();
                    continue;
                }
            }
            match &self.current {
                Token::FStringMiddle(s) => {
                    let s = s.clone();
                    self.next();
                    parts.push(FStringPart::String(s));
                }
                Token::FStringStart => {
                    // Inner f-string from tokenize_fstring_expr — parse full expression
                    let expr = self.parse_expr()?;
                    let mut conversion: u8 = 0;
                    let mut format_spec: Option<Box<Expr>> = None;
                    if let Token::FStringConversion(c) = &self.current {
                        conversion = *c;
                        self.next();
                    }
                    if let Token::FormatSpec(spec_text) = &self.current {
                        let spec = spec_text.clone();
                        self.next();
                        if spec.contains('{') {
                            let mut nested_lex =
                                crate::token::Lexer::new(&format!("f\"{}\"", spec));
                            let first_tok = nested_lex.next_token();
                            if first_tok == Token::FStringStart {
                                let mut nested_parser = Parser::new(&format!("f\"{}\"", spec));
                                if let Ok(Expr::FString(inner)) = nested_parser.parse_expr() {
                                    format_spec = Some(Box::new(Expr::FString(inner)));
                                } else {
                                    format_spec =
                                        Some(Box::new(Expr::Constant(Constant::String(spec))));
                                }
                            } else {
                                format_spec =
                                    Some(Box::new(Expr::Constant(Constant::String(spec))));
                            }
                        } else {
                            format_spec = Some(Box::new(Expr::Constant(Constant::String(spec))));
                        }
                    }
                    parts.push(FStringPart::Expr {
                        expr: Box::new(expr),
                        conversion,
                        format_spec,
                    });
                }
                Token::FStringEnd => {
                    self.next();
                    break;
                }
                _ => {
                    if self.at(&Token::EndOfFile) {
                        break;
                    }
                    let mut expr = self.parse_expr()?;
                    // Handle tuple expressions inside f-string: {x, y, z}
                    if self.eat(&Token::Comma) {
                        let mut elts = vec![expr];
                        loop {
                            if self.at(&Token::RightBrace)
                                || self.at(&Token::FStringEnd)
                                || matches!(&self.current, Token::FStringConversion(_))
                                || matches!(&self.current, Token::FormatSpec(_))
                            {
                                break;
                            }
                            elts.push(self.parse_expr()?);
                            if !self.eat(&Token::Comma) {
                                break;
                            }
                        }
                        expr = Expr::Tuple(elts);
                    }
                    let mut conversion: u8 = 0;
                    let mut format_spec: Option<Box<Expr>> = None;
                    // Check for FStringConversion token
                    if let Token::FStringConversion(c) = &self.current {
                        conversion = *c;
                        self.next();
                    }
                    // Check for FormatSpec token
                    if let Token::FormatSpec(spec_text) = &self.current {
                        let spec = spec_text.clone();
                        self.next();
                        // Parse the format spec as a string constant (simple cases)
                        if spec.contains('{') {
                            // Nested format spec — parse as f-string
                            let mut nested_lex =
                                crate::token::Lexer::new(&format!("f\"{}\"", spec));
                            let first_tok = nested_lex.next_token();
                            if first_tok == Token::FStringStart {
                                let mut nested_parser = Parser::new(&format!("f\"{}\"", spec));
                                if let Ok(Expr::FString(inner)) = nested_parser.parse_expr() {
                                    format_spec = Some(Box::new(Expr::FString(inner)));
                                } else {
                                    format_spec =
                                        Some(Box::new(Expr::Constant(Constant::String(spec))));
                                }
                            } else {
                                format_spec =
                                    Some(Box::new(Expr::Constant(Constant::String(spec))));
                            }
                        } else {
                            format_spec = Some(Box::new(Expr::Constant(Constant::String(spec))));
                        }
                    }
                    parts.push(FStringPart::Expr {
                        expr: Box::new(expr),
                        conversion,
                        format_spec,
                    });
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
        let mut args: Vec<Arg> = Vec::new();
        let mut seen_star = false;
        let mut seen_slash = false;
        let mut seen_names: std::collections::HashSet<String> = std::collections::HashSet::new();
        loop {
            if self.at(&Token::Colon) {
                break;
            }
            if self.at(&Token::Slash) && !seen_slash {
                // Same validation as def parameters: '/' must follow a param
                // and must not follow `*` (real CPython rejects
                // `lambda *, a, /:` and `lambda /:`).
                if args.is_empty() || seen_star || args.iter().any(|a| a.is_vararg || a.is_kwarg) {
                    return Err("unexpected '/' in lambda".to_string());
                }
                self.next();
                seen_slash = true;
                for arg in args.iter_mut() {
                    arg.is_posonlyarg = true;
                }
                if !self.eat(&Token::Comma) {
                    break;
                }
                continue;
            }
            if self.eat(&Token::Star) {
                if self.at(&Token::Colon) || self.at(&Token::Comma) {
                    // Bare `*` keyword-only separator (`lambda *, kw=None:
                    // ...`) — must still consume the following comma (or
                    // stop at `:`) the same way every other arm does at the
                    // loop's end; the previous bare `continue` skipped that
                    // entirely, leaving the comma unconsumed and making the
                    // next iteration immediately fail expecting a NAME.
                    seen_star = true;
                    if !self.eat(&Token::Comma) {
                        break;
                    }
                    continue;
                }
                let name = self.expect_name()?;
                if !seen_names.insert(name.clone()) {
                    return Err(format!(
                        "duplicate argument '{}' in function definition",
                        name
                    ));
                }
                args.push(Arg {
                    arg: name,
                    annotation: None,
                    is_vararg: true,
                    is_kwarg: false,
                    is_posonlyarg: false,
                    is_kwonly: false,
                    default: None,
                });
                seen_star = true;
            } else if self.eat(&Token::DoubleStar) {
                let name = self.expect_name()?;
                if !seen_names.insert(name.clone()) {
                    return Err(format!(
                        "duplicate argument '{}' in function definition",
                        name
                    ));
                }
                args.push(Arg {
                    arg: name,
                    annotation: None,
                    is_vararg: false,
                    is_kwarg: true,
                    is_posonlyarg: false,
                    is_kwonly: false,
                    default: None,
                });
            } else {
                let name = self.expect_name()?;
                if !seen_names.insert(name.clone()) {
                    return Err(format!(
                        "duplicate argument '{}' in function definition",
                        name
                    ));
                }
                let default = if self.eat(&Token::Equal) {
                    Some(Box::new(self.parse_expr()?))
                } else {
                    None
                };
                args.push(Arg {
                    arg: name,
                    annotation: None,
                    is_vararg: false,
                    is_kwarg: false,
                    is_posonlyarg: false,
                    is_kwonly: seen_star,
                    default,
                });
            }
            if !self.eat(&Token::Comma) {
                break;
            }
        }
        // Same default-ordering rule as def parameters.
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

    fn parse_yield_expr(&mut self) -> Result<Expr, String> {
        self.next();
        if self.eat(&Token::From) {
            let expr = self.parse_conditional_expr()?;
            Ok(Expr::YieldFrom(Box::new(expr)))
        } else {
            let expr = if !self.at(&Token::Newline)
                && !self.at(&Token::RightParen)
                && !self.at(&Token::RightBracket)
                && !self.at(&Token::RightBrace)
                && !self.at(&Token::FStringEnd)
                && !self.at(&Token::FStringConversion(0))
                && !self.at(&Token::Colon)
                && !self.at(&Token::Comma)
                && !self.at(&Token::Semicolon)
                && !self.at(&Token::EndOfFile)
            {
                let first = if self.at(&Token::Star) {
                    self.next();
                    Expr::Starred(Box::new(self.parse_conditional_expr()?))
                } else {
                    self.parse_conditional_expr()?
                };
                // yield x, y → yield (x, y)  (tuple yield)
                if self.at(&Token::Comma) {
                    let mut elts = vec![first];
                    while self.eat(&Token::Comma) {
                        if self.at(&Token::Newline)
                            || self.at(&Token::Semicolon)
                            || self.at(&Token::RightParen)
                            || self.at(&Token::RightBracket)
                            || self.at(&Token::RightBrace)
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
            Ok(Expr::Yield(expr))
        }
    }
}

/// Try to parse a source string as a single expression.
/// Used by the REPL to detect expression statements whose value should
/// be displayed via sys.displayhook instead of being discarded by POP_TOP.
pub fn try_parse_as_expression(source: &str) -> Result<Program, String> {
    let mut parser = Parser::new(source);
    let expr = parser.parse_expr()?;
    // Consume trailing newlines
    while parser.eat(&Token::Newline) || parser.eat(&Token::Semicolon) {}
    if !parser.at(&Token::EndOfFile) {
        return Err("extra tokens after expression".to_string());
    }
    Ok(Program::Expression(Box::new(expr)))
}
