use crate::ast::*;
use crate::token::Token;

use super::Parser;

/// Merge a plain-string chunk into f-string part accumulation,
/// folding into a trailing String part when adjacent (mirrors
/// CPython: `f'a' 'b' f'c'` is one `Constant('abc')` inside the
/// JoinedStr).
fn accum_fstring_str(parts: &mut Vec<FStringPart>, s: crate::object::pystring::PyStr, span: Span) {
    // Empty strings contribute no part (mirrors CPython: `f'{x}' ''`
    // is `JoinedStr([FormattedValue])`).
    if s.as_str().is_empty() {
        return;
    }
    if let Some(FStringPart::String(prev, pspan)) = parts.last_mut() {
        *prev = crate::object::pystring::PyStr::concat_wtf8(&prev.clone(), &s);
        pspan.1 = span.1;
    } else {
        parts.push(FStringPart::String(s, span));
    }
}

/// Extend accumulation with a parsed f-string's parts (boundary
/// String parts fold together like above).
fn accum_fstring_parts(parts: &mut Vec<FStringPart>, fstr: Expr) {
    let inner = match fstr.into_unlocated() {
        Expr::FString(inner) => inner,
        // `parse_fstring` always builds `FString` on success.
        _ => unreachable!("f-string parse produced a non-FString node"),
    };
    for p in inner {
        match p {
            FStringPart::String(s, span) => accum_fstring_str(parts, s, span),
            other => parts.push(other),
        }
    }
}

/// Merge a parsed t-string's parts into template accumulation,
/// folding adjacent String parts (mirrors CPython: `t'a' t'b'` joins
/// to one strings tuple).
fn accum_tstring_parts(parts: &mut Vec<TemplateStrPart>, tstr: Expr) {
    let inner = match tstr.into_unlocated() {
        Expr::TemplateStr(inner) => inner,
        // `parse_tstring` always builds `TemplateStr` on success.
        _ => unreachable!("t-string parse produced a non-TemplateStr node"),
    };
    for p in inner {
        match p {
            TemplateStrPart::String(s, span) => {
                // Middle text is char-built (always valid UTF-8), and
                // plain strings can no longer neighbor t-strings, so
                // plain `push_str` is lossless here.
                if let Some(TemplateStrPart::String(prev, pspan)) = parts.last_mut() {
                    prev.push_str(&s);
                    pspan.1 = span.1;
                } else {
                    parts.push(TemplateStrPart::String(s, span));
                }
            }
            other => parts.push(other),
        }
    }
}

/// Flush part accumulation into an expression: pure-string runs keep
/// the historical BinOp/Constant shape; anything involving an f-string
/// becomes a single `FString` (mirrors CPython's JoinedStr merging).
fn flush_fstring_parts(parts: Vec<FStringPart>, saw_fstring: bool) -> Expr {
    if parts.is_empty() && !saw_fstring {
        // Only possible when every chunk was an empty plain string
        // (`''` alone collapses to one empty Constant). An empty
        // f-string (`f''`) still yields `FString([])` below.
        return Expr::Constant(Constant::String(crate::object::pystring::PyStr::from_str(
            "",
        )));
    }
    if !saw_fstring {
        let mut it = parts.into_iter().map(|p| match p {
            FStringPart::String(s, _) => Expr::Constant(Constant::String(s)),
            _ => unreachable!("pure-string accumulation holds only Strings"),
        });
        let first = it.next().unwrap();
        return it.fold(first, |a, b| Expr::BinOp {
            left: Box::new(a),
            op: Operator::Add,
            right: Box::new(b),
        });
    }
    Expr::FString(parts)
}

impl Parser {
    pub(crate) fn parse_atom(&mut self) -> Result<Expr, String> {
        // Stamp source spans on every expression node (see
        // `Expr::Located`): start = current token, end =
        // lookahead token start (approximation). Transparent
        // parentheses keep the INNER span (mirrors CPython: `(a)`
        // positions `a`, not `(`) — but displays (`(a, b)`) keep
        // the outer one.
        // Start of the current token (tracked through
        // lookahead scans in `current_start`).
        let (sl, sc) = self.current_start;
        let e = self.parse_atom_inner()?;
        if self.group_paren && !matches!(e.unlocate(), Expr::Tuple(_) | Expr::List(_)) {
            return Ok(e);
        }
        // Wrap only freshly built (unpositioned) nodes; pass through
        // already-spanned ones (pure delegation keeps inner spans).
        if e.position().is_none() {
            // End of the last consumed token (tracked via
            // `prev_end`, not the lookahead start).
            let (el, ec) = self.prev_end;
            Ok(Expr::located(sl, sc, el, ec, e))
        } else {
            Ok(e)
        }
    }

    fn parse_atom_inner(&mut self) -> Result<Expr, String> {
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
                let s = s.clone();
                let sstart = self.current_start;
                self.next();
                let send = self.prev_end;
                // Implicit string concatenation: adjacent String/FString
                // parts accumulate into ONE JoinedStr (mirrors CPython:
                // `'a' f'{x}'` is a single JoinedStr). Pure-string runs
                // keep the historical BinOp/Constant shape; TString
                // involvement falls back to BinOp chaining below.
                // Empty strings contribute no part at all (mirrors
                // CPython: `f'{x}' ''` is `JoinedStr([FormattedValue])`,
                // not `JoinedStr([FV, Constant('')])`).
                let mut fparts: Vec<FStringPart> = Vec::new();
                if !s.as_str().is_empty() {
                    fparts.push(FStringPart::String(s, (sstart, send)));
                }
                let mut saw_fstring = false;
                loop {
                    // NOTE: newlines are NOT eaten here — implicit
                    // concatenation never crosses a statement boundary
                    // (mirrors CPython: `f'a'\nf'b'` parses as two
                    // statements, not one concatenation; newlines inside
                    // brackets never surface as tokens anyway).
                    match &self.current {
                        Token::String(s2) => {
                            let s = s2.clone();
                            let sstart = self.current_start;
                            self.next();
                            let send = self.prev_end;
                            accum_fstring_str(&mut fparts, s, (sstart, send));
                        }
                        Token::FStringStart => {
                            self.next();
                            let fstr = self.parse_fstring()?;
                            accum_fstring_parts(&mut fparts, fstr);
                            saw_fstring = true;
                        }
                        Token::TStringStart => {
                            // Mixing t-strings with text is an error
                            // (mirrors CPython: `'a' t'{x}'`).
                            return Err(
                                "cannot mix t-string literals with string or bytes literals"
                                    .to_string(),
                            );
                        }
                        // Mixing bytes with text in implicit
                        // concatenation is an error (mirrors CPython:
                        // `f'' b''`).
                        Token::Bytes(_) => {
                            return Err("cannot mix bytes and nonbytes literals".to_string());
                        }
                        _ => break,
                    }
                }
                Ok(flush_fstring_parts(fparts, saw_fstring))
            }

            Token::Bytes(b) => {
                let mut parts = vec![b.clone()];
                self.next();
                // Implicit bytes concatenation: adjacent bytes with optional newlines
                loop {
                    // NOTE: no newline eating (see above).
                    if !matches!(&self.current, Token::Bytes(_)) {
                        break;
                    }
                    if let Token::Bytes(next) = &self.current {
                        parts.push(next.clone());
                        self.next();
                    }
                }
                // Bytes adjacent to text can never concatenate
                // (mirrors CPython: `b'' f''`). A t-string neighbor
                // reports the t-string mixing error instead.
                if matches!(&self.current, Token::TStringStart) {
                    return Err(
                        "cannot mix t-string literals with string or bytes literals".to_string()
                    );
                }
                if matches!(&self.current, Token::String(_) | Token::FStringStart) {
                    return Err("cannot mix bytes and nonbytes literals".to_string());
                }
                let combined: Vec<u8> = parts.into_iter().flatten().collect();
                Ok(Expr::Constant(Constant::Bytes(combined)))
            }

            Token::FStringStart => {
                self.next();
                // Like the String arm: adjacent String/FString parts
                // merge into ONE JoinedStr. TString involvement is an
                // error (mirrors CPython).
                let mut fparts: Vec<FStringPart> = Vec::new();
                accum_fstring_parts(&mut fparts, self.parse_fstring()?);
                loop {
                    match &self.current {
                        Token::FStringStart => {
                            self.next();
                            let fstr = self.parse_fstring()?;
                            accum_fstring_parts(&mut fparts, fstr);
                        }
                        Token::TStringStart => {
                            return Err(
                                "cannot mix t-string literals with string or bytes literals"
                                    .to_string(),
                            );
                        }
                        Token::String(s) => {
                            let s = s.clone();
                            let sstart = self.current_start;
                            self.next();
                            let send = self.prev_end;
                            accum_fstring_str(&mut fparts, s, (sstart, send));
                        }
                        Token::Bytes(_) => {
                            return Err("cannot mix bytes and nonbytes literals".to_string());
                        }
                        _ => break,
                    }
                }
                Ok(flush_fstring_parts(fparts, true))
            }

            Token::TStringStart => {
                self.next();
                // Adjacent t-strings merge into ONE Template (mirrors
                // CPython: `t'a' t'b'` has a single joined strings
                // tuple). Anything else adjacent is a mixing error.
                let mut tparts: Vec<TemplateStrPart> = Vec::new();
                accum_tstring_parts(&mut tparts, self.parse_tstring()?);
                loop {
                    match &self.current {
                        Token::TStringStart => {
                            self.next();
                            accum_tstring_parts(&mut tparts, self.parse_tstring()?);
                        }
                        Token::FStringStart | Token::String(_) | Token::Bytes(_) => {
                            return Err(
                                "cannot mix t-string literals with string or bytes literals"
                                    .to_string(),
                            );
                        }
                        _ => break,
                    }
                }
                Ok(Expr::TemplateStr(tparts))
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
                    let mut had_comma = false;
                    while !self.at(&Token::RightParen) && !self.at(&Token::EndOfFile) {
                        if self.at(&Token::Star) {
                            self.next();
                            elts.push(Expr::Starred(Box::new(self.parse_expr()?)));
                            // `(*x for ...)` — CPython's `invalid_group` rule
                            // rejects this with "cannot use starred expression here".
                            if self.at(&Token::For) || self.at(&Token::Async) {
                                return Err("cannot use starred expression here".to_string());
                            }
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
                        if self.eat(&Token::Comma) {
                            had_comma = true;
                        } else {
                            break;
                        }
                    }
                    self.expect(&Token::RightParen)?;
                    // `(*x)` without trailing comma is a parenthesized
                    // starred expression — CPython's `invalid_group` rule
                    // rejects it with "cannot use starred expression here".
                    if elts.len() == 1 && matches!(&elts[0], Expr::Starred(_)) && !had_comma {
                        return Err("cannot use starred expression here".to_string());
                    }
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
                        // Plain parenthesized group: mark it so `and`/`or`
                        // chains don't flatten across the parens.
                        self.group_paren = true;
                        first
                    }
                } else {
                    let first = self.parse_expr()?;
                    if let Some(is_async) = self.eat_comp_for(true)? {
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
                        while let Some(is_async) = self.eat_comp_for(true)? {
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
                        while let Some(is_async) = self.eat_comp_for(true)? {
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
                        // Plain parenthesized group: mark it so `and`/`or`
                        // chains don't flatten across the parens.
                        self.group_paren = true;
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
                    // CPython's `invalid_comprehension` rule rejects
                    // starred expressions in comprehensions.
                    if matches!(&elts[0], Expr::Starred(_)) {
                        return Err(
                            "iterable unpacking cannot be used in comprehension".to_string()
                        );
                    }
                    let is_async = self.eat_comp_for(false)?.unwrap();
                    let target = self.parse_for_target()?;
                    self.expect(&Token::In)?;
                    let iter = self.parse_or_expr()?;
                    let mut generators = vec![Comprehension {
                        target: Box::new(target),
                        iter: Box::new(iter),
                        ifs: Vec::new(),
                        is_async,
                    }];
                    while let Some(is_async) = self.eat_comp_for(false)? {
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
                    while let Some(is_async) = self.eat_comp_for(false)? {
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
                        // CPython's `invalid_dict_comprehension` rule rejects
                        // dict unpacking in dict comprehensions.
                        if self.at(&Token::For) || self.at(&Token::Async) {
                            return Err(
                                "dict unpacking cannot be used in dict comprehension".to_string()
                            );
                        }
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
                            if let Some(is_async) = self.eat_comp_for(false)? {
                                let target = self.parse_for_target()?;
                                self.expect(&Token::In)?;
                                let iter = self.parse_or_expr()?;
                                let mut generators = vec![Comprehension {
                                    target: Box::new(target),
                                    iter: Box::new(iter),
                                    ifs: Vec::new(),
                                    is_async,
                                }];
                                while let Some(is_async) = self.eat_comp_for(false)? {
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
                                while let Some(is_async) = self.eat_comp_for(false)? {
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
                            if let Some(is_async) = self.eat_comp_for(false)? {
                                // CPython's `invalid_comprehension` rule rejects
                                // starred expressions in comprehensions.
                                if let Some(last) = values.last() {
                                    if matches!(last, Expr::Starred(_)) {
                                        return Err(
                                            "iterable unpacking cannot be used in comprehension"
                                                .to_string(),
                                        );
                                    }
                                }
                                let target = self.parse_for_target()?;
                                self.expect(&Token::In)?;
                                let iter = self.parse_or_expr()?;
                                let mut generators = vec![Comprehension {
                                    target: Box::new(target),
                                    iter: Box::new(iter),
                                    ifs: Vec::new(),
                                    is_async,
                                }];
                                while let Some(is_async) = self.eat_comp_for(false)? {
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
                                while let Some(is_async) = self.eat_comp_for(false)? {
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
                if self.async_depth == 0
                    && (self.fn_depth > 0 || !self.allow_top_level_await)
                    && self.genexpr_depth == 0
                {
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
        // Cap interpolated-string nesting (mirrors CPython's
        // MAXFSTRINGLEVEL: "too many nested f-strings or t-strings").
        // Errors abort the whole parse, so only success decrements
        // (sequential f-strings must not accumulate).
        // The string starts at its opener, recorded on consumption
        // (a LIFO stack, since nested strings complete first). Falls
        // back to the lexer's prefix record, which only ever lags
        // for the outermost string.
        let (sl, sc) = self
            .string_starts
            .pop()
            .unwrap_or_else(|| self.lexer.fstring_start_pos());
        self.fstring_depth += 1;
        if self.fstring_depth > 150 {
            return Err("too many nested f-strings or t-strings".to_string());
        }
        let result = self.parse_fstring_inner();
        self.fstring_depth -= 1;
        // End of the last consumed token (tracked via
        // `prev_end`, not the lookahead start).
        let (el, ec) = self.prev_end;
        return result.map(|e| Expr::located(sl, sc, el, ec, e));
    }

    fn parse_fstring_inner(&mut self) -> Result<Expr, String> {
        let mut parts = Vec::new();
        // Pending source spans from `FStringSpan` markers (pushed just
        // before each literal/field's tokens): consumed when building
        // the corresponding part below.
        let mut pending_lit: Option<((usize, usize), (usize, usize))> = None;
        let mut pending_field: Option<((usize, usize), (usize, usize))> = None;
        loop {
            match &self.current {
                Token::FStringSpan {
                    open,
                    close,
                    is_field,
                } => {
                    let span = (*open, *close);
                    if *is_field {
                        pending_field = Some(span);
                    } else {
                        pending_lit = Some(span);
                    }
                    self.next();
                }
                Token::FStringMiddle(s) => {
                    let s = s.clone();
                    self.next();
                    // Middle text is built from chars (always valid
                    // UTF-8), so `from_str` is lossless here.
                    parts.push(FStringPart::String(
                        crate::object::pystring::PyStr::from_str(&s),
                        pending_lit.take().unwrap_or(((1, 1), (1, 1))),
                    ));
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
                                crate::token::Lexer::new(&format!("f\"{}\"", spec), false);
                            let first_tok = nested_lex.next_token();
                            if first_tok == Token::FStringStart {
                                let mut nested_parser =
                                    Parser::new(&format!("f\"{}\"", spec), false);
                                // Peel positions: the nested parse stamps
                                // `Located` (see `Expr::Located`).
                                match nested_parser.parse_expr().map(Expr::into_unlocated) {
                                    Ok(Expr::FString(inner)) => {
                                        format_spec = Some(Box::new(Expr::FString(inner)));
                                    }
                                    _ => {
                                        format_spec = Some(Box::new(Expr::Constant(
                                            Constant::String(spec.into()),
                                        )));
                                    }
                                }
                            } else {
                                format_spec =
                                    Some(Box::new(Expr::Constant(Constant::String(spec.into()))));
                            }
                        } else {
                            format_spec =
                                Some(Box::new(Expr::Constant(Constant::String(spec.into()))));
                        }
                    }
                    parts.push(FStringPart::Expr {
                        expr: Box::new(expr),
                        conversion,
                        format_spec,
                        span: pending_field.take().unwrap_or(((1, 1), (1, 1))),
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
                    // A leading `*` parses as Starred (valid in tuple
                    // displays like `{*x,}`; a bare one is rejected
                    // below with CPython's codegen message). A leading
                    // `**` can never start a field expression.
                    let mut expr = if self.at(&Token::Star) {
                        self.next();
                        Expr::Starred(Box::new(self.parse_expr()?))
                    } else if self.at(&Token::DoubleStar) {
                        return Err("f-string: expecting a valid expression after '{'".to_string());
                    } else {
                        self.parse_expr()?
                    };
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
                    // A bare Starred outside a tuple display is rejected
                    // (mirrors CPython codegen's "can't use starred
                    // expression here", e.g. `f'{*a}'`).
                    if matches!(&expr, Expr::Starred(_)) {
                        return Err("can't use starred expression here".to_string());
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
                                crate::token::Lexer::new(&format!("f\"{}\"", spec), false);
                            let first_tok = nested_lex.next_token();
                            if first_tok == Token::FStringStart {
                                let mut nested_parser =
                                    Parser::new(&format!("f\"{}\"", spec), false);
                                // Peel positions: the nested parse stamps
                                // `Located` (see `Expr::Located`).
                                match nested_parser.parse_expr().map(Expr::into_unlocated) {
                                    Ok(Expr::FString(inner)) => {
                                        format_spec = Some(Box::new(Expr::FString(inner)));
                                    }
                                    _ => {
                                        format_spec = Some(Box::new(Expr::Constant(
                                            Constant::String(spec.into()),
                                        )));
                                    }
                                }
                            } else {
                                format_spec =
                                    Some(Box::new(Expr::Constant(Constant::String(spec.into()))));
                            }
                        } else {
                            format_spec =
                                Some(Box::new(Expr::Constant(Constant::String(spec.into()))));
                        }
                    }
                    parts.push(FStringPart::Expr {
                        expr: Box::new(expr),
                        conversion,
                        format_spec,
                        span: pending_field.take().unwrap_or(((1, 1), (1, 1))),
                    });
                }
            }
        }
        Ok(Expr::FString(parts))
    }

    /// Template strings (PEP 750): mirrors `parse_fstring`, but each
    /// interpolation keeps its raw source text (`Interpolation.str`),
    /// delivered by the lexer as `Token::TStringExpr`.
    pub(crate) fn parse_tstring(&mut self) -> Result<Expr, String> {
        // Same nesting cap as f-strings (CPython's message covers
        // both: "too many nested f-strings or t-strings").
        // Starts at the opener like f-strings (see above).
        let (sl, sc) = self
            .string_starts
            .pop()
            .unwrap_or_else(|| self.lexer.fstring_start_pos());
        self.fstring_depth += 1;
        if self.fstring_depth > 150 {
            return Err("too many nested f-strings or t-strings".to_string());
        }
        let result = self.parse_tstring_inner();
        self.fstring_depth -= 1;
        // End of the last consumed token (tracked via
        // `prev_end`, not the lookahead start).
        let (el, ec) = self.prev_end;
        return result.map(|e| Expr::located(sl, sc, el, ec, e));
    }

    fn parse_tstring_inner(&mut self) -> Result<Expr, String> {
        let mut parts = Vec::new();
        // Pending source spans from `FStringSpan` markers (same
        // mechanism as `parse_fstring_inner`).
        let mut pending_lit: Option<((usize, usize), (usize, usize))> = None;
        let mut pending_field: Option<((usize, usize), (usize, usize))> = None;
        loop {
            match &self.current {
                Token::FStringSpan {
                    open,
                    close,
                    is_field,
                } => {
                    let span = (*open, *close);
                    if *is_field {
                        pending_field = Some(span);
                    } else {
                        pending_lit = Some(span);
                    }
                    self.next();
                }
                Token::TStringMiddle(s) => {
                    let s = s.clone();
                    self.next();
                    parts.push(TemplateStrPart::String(
                        s,
                        pending_lit.take().unwrap_or(((1, 1), (1, 1))),
                    ));
                }
                Token::TStringExpr(raw) => {
                    let str_text = raw.clone();
                    self.next();
                    let (expr, conversion, format_spec) = self.parse_tstring_value()?;
                    parts.push(TemplateStrPart::Interpolation {
                        expr: Box::new(expr),
                        str_text,
                        conversion,
                        format_spec,
                        span: pending_field.take().unwrap_or(((1, 1), (1, 1))),
                    });
                }
                Token::TStringEnd => {
                    self.next();
                    break;
                }
                _ => {
                    if self.at(&Token::EndOfFile) {
                        break;
                    }
                    // Fallback: value without recorded source text
                    // (unreachable with the current lexer, which always
                    // emits TStringExpr before interpolation tokens).
                    let (expr, conversion, format_spec) = self.parse_tstring_value()?;
                    parts.push(TemplateStrPart::Interpolation {
                        expr: Box::new(expr),
                        str_text: String::new(),
                        conversion,
                        format_spec,
                        span: pending_field.take().unwrap_or(((1, 1), (1, 1))),
                    });
                }
            }
        }
        Ok(Expr::TemplateStr(parts))
    }

    /// Parse one interpolation value + optional `!conversion` / `:spec`.
    fn parse_tstring_value(&mut self) -> Result<(Expr, u8, Option<Box<Expr>>), String> {
        // Leading `*` parses as Starred (tuple displays like `{*x,}`
        // stay valid); a bare one is rejected below. `**` can never
        // start an interpolation.
        let mut expr = if self.at(&Token::Star) {
            self.next();
            Expr::Starred(Box::new(self.parse_expr()?))
        } else if self.at(&Token::DoubleStar) {
            return Err("t-string: expecting a valid expression after '{'".to_string());
        } else {
            self.parse_expr()?
        };
        // Handle tuple expressions inside interpolation: {x, y, z}
        if self.eat(&Token::Comma) {
            let mut elts = vec![expr];
            loop {
                if self.at(&Token::RightBrace)
                    || self.at(&Token::TStringEnd)
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
        // A bare Starred outside a tuple display is rejected (mirrors
        // CPython codegen's "can't use starred expression here").
        if matches!(&expr, Expr::Starred(_)) {
            return Err("can't use starred expression here".to_string());
        }
        let mut conversion: u8 = 0;
        if let Token::FStringConversion(c) = &self.current {
            conversion = *c;
            self.next();
        }
        let mut format_spec: Option<Box<Expr>> = None;
        if let Token::FormatSpec(spec_text) = &self.current {
            let spec = spec_text.clone();
            self.next();
            // Format specs render as JoinedStr (same as f-strings).
            if spec.contains('{') {
                let mut nested_lex = crate::token::Lexer::new(&format!("f\"{}\"", spec), false);
                let first_tok = nested_lex.next_token();
                if first_tok == Token::FStringStart {
                    let mut nested_parser = Parser::new(&format!("f\"{}\"", spec), false);
                    // Peel positions (see above).
                    match nested_parser.parse_expr().map(Expr::into_unlocated) {
                        Ok(Expr::FString(inner)) => {
                            format_spec = Some(Box::new(Expr::FString(inner)));
                        }
                        _ => {
                            format_spec =
                                Some(Box::new(Expr::Constant(Constant::String(spec.into()))));
                        }
                    }
                } else {
                    format_spec = Some(Box::new(Expr::Constant(Constant::String(spec.into()))));
                }
            } else {
                format_spec = Some(Box::new(Expr::Constant(Constant::String(spec.into()))));
            }
        }
        Ok((expr, conversion, format_spec))
    }

    pub(crate) fn parse_lambda(&mut self) -> Result<Expr, String> {
        // Stamp source spans on every expression node (see
        // `Expr::Located`): start = current token, end =
        // lookahead token start (approximation).
        // Start of the current token (tracked through
        // lookahead scans in `current_start`).
        let (sl, sc) = self.current_start;
        let e = self.parse_lambda_inner()?;
        // Wrap only freshly built (unpositioned) nodes; pass through
        // already-spanned ones (pure delegation and transparent
        // parens keep inner spans).
        if e.position().is_none() {
            // End of the last consumed token (tracked via
            // `prev_end`, not the lookahead start).
            let (el, ec) = self.prev_end;
            Ok(Expr::located(sl, sc, el, ec, e))
        } else {
            Ok(e)
        }
    }

    fn parse_lambda_inner(&mut self) -> Result<Expr, String> {
        self.next();
        let args = if self.eat(&Token::Colon) {
            Vec::new()
        } else {
            let args = self.parse_lambda_args()?;
            self.expect(&Token::Colon)?;
            args
        };
        self.fn_depth += 1;
        let body = self.parse_expr()?;
        self.fn_depth -= 1;
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
        // Stamp source spans on every expression node (see
        // `Expr::Located`): start = current token, end =
        // lookahead token start (approximation).
        // Start of the current token (tracked through
        // lookahead scans in `current_start`).
        let (sl, sc) = self.current_start;
        let e = self.parse_yield_expr_inner()?;
        // Wrap only freshly built (unpositioned) nodes; pass through
        // already-spanned ones (pure delegation and transparent
        // parens keep inner spans).
        if e.position().is_none() {
            // End of the last consumed token (tracked via
            // `prev_end`, not the lookahead start).
            let (el, ec) = self.prev_end;
            Ok(Expr::located(sl, sc, el, ec, e))
        } else {
            Ok(e)
        }
    }

    fn parse_yield_expr_inner(&mut self) -> Result<Expr, String> {
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
