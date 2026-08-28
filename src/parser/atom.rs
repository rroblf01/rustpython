use crate::ast::*;
use crate::token::Token;

use super::Parser;

impl Parser {
    pub(crate) fn parse_atom(&mut self) -> Result<Expr, String> {
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
                        self.genexpr_depth += 1;
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
                        self.genexpr_depth -= 1;
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
                // `await` at top level is SyntaxError unless the
                // PyCF_ALLOW_TOP_LEVEL_AWAIT flag is set (test_builtin's
                // test_compile_top_level_await). Inside a generator
                // expression it's always legal (async-generator semantics,
                // test_asyncgen's make_arange).
                if self.async_depth == 0 && !self.allow_top_level_await && self.genexpr_depth == 0 {
                    return Err("'await' outside async function".to_string());
                }
                self.next();
                let expr = self.parse_unary()?;
                Ok(Expr::Await(Box::new(expr)))
            }

            _ => unexpected_token!(self, "expression"),
        }
    }

    pub(crate) fn parse_fstring(&mut self) -> Result<Expr, String> {
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

    pub(crate) fn parse_lambda(&mut self) -> Result<Expr, String> {
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

    pub(crate) fn parse_lambda_args(&mut self) -> Result<Vec<Arg>, String> {
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

    pub(crate) fn parse_yield_expr(&mut self) -> Result<Expr, String> {
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
