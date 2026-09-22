use super::unicode_name_to_char;
use super::{FStringPartScan, Lexer, Token};
use unicode_normalization::UnicodeNormalization;

impl Lexer {
    pub fn next_token(&mut self) -> Token {
        // Emit any lexer-detected error (e.g. unindent does not match).
        if let Some(err) = self.pending_error.take() {
            return Token::LexerError(err);
        }
        // If we have pending f-string expression tokens, return them
        // (with their recorded spans, popped in lockstep).
        if let Some(tok) = self.fstring_expr_pending.pop() {
            if let Some(pos) = self.fstring_expr_pos.pop() {
                self.tok_start = pos;
            }
            if let Some(end) = self.fstring_expr_end.pop() {
                self.tok_end = end;
            }
            return tok;
        }
        // Check if we're in the middle of emitting f-string/t-string parts
        if let Some(_quote) = self.fstring_quote {
            if self.fstring_part_idx < self.fstring_parts.len() {
                let part = self.fstring_parts[self.fstring_part_idx].clone();
                self.fstring_part_idx += 1;
                let is_tstring = self.fstring_is_tstring;
                // f-string start fallback for synthesized tokens whose
                // own position is untested (conversions, end).
                let fb = self.fstring_fallback_pos();
                let close_end = self.fstring_close_end;
                // to_push holds (token, start, end) in OUTPUT order.
                let mut to_push: Vec<(Token, (usize, usize), (usize, usize))> = Vec::new();
                // Literal span marker + text (if any) — FIRST.
                if !part.literal.is_empty() {
                    let lit_end = self.advance_pos(part.lit_start, &part.literal);
                    to_push.push((
                        Token::FStringSpan {
                            open: part.lit_start,
                            close: lit_end,
                            is_field: false,
                        },
                        part.lit_start,
                        lit_end,
                    ));
                    if is_tstring {
                        to_push.push((
                            Token::TStringMiddle(part.literal.clone()),
                            part.lit_start,
                            lit_end,
                        ));
                    } else {
                        to_push.push((
                            Token::FStringMiddle(part.literal.clone()),
                            part.lit_start,
                            lit_end,
                        ));
                    }
                }
                // Field span marker + expression tokens (if any).
                if !part.expr_text.is_empty() {
                    to_push.push((
                        Token::FStringSpan {
                            open: part.field_open,
                            close: part.field_end,
                            is_field: true,
                        },
                        part.field_open,
                        part.field_end,
                    ));
                    if is_tstring {
                        // The raw expression text becomes CPython's
                        // `Interpolation.str` field, with trailing
                        // whitespace stripped (mirrors CPython:
                        // `t"{value = }"` records `'value'`).
                        let expr_end = self.advance_pos(part.field_start, &part.expr_text);
                        let trimmed = part.expr_text.trim_end().to_string();
                        to_push.push((Token::TStringExpr(trimmed), part.field_start, expr_end));
                    }
                    let (expr_tokens, expr_pos, expr_end) =
                        self.tokenize_fstring_expr(&part.expr_text, part.field_start);
                    to_push.extend(
                        expr_tokens
                            .into_iter()
                            .zip(expr_pos)
                            .zip(expr_end)
                            .map(|((t, s), e)| (t, s, e)),
                    );
                    // If there's a conversion, push the FStringConversion token
                    if part.conversion != 0 {
                        to_push.push((Token::FStringConversion(part.conversion), fb, fb));
                    }
                    // If there's a format spec, push the FormatSpec token
                    // (an explicit-but-empty `:` still counts: `f'{a:}'`
                    // has `format_spec=JoinedStr([])` in the AST).
                    if part.has_spec {
                        let spec_end = self.advance_pos(part.spec_start, &part.format_spec);
                        to_push.push((
                            Token::FormatSpec(part.format_spec.clone()),
                            part.spec_start,
                            spec_end,
                        ));
                    }
                }
                // If this is the last part, end the f-string/t-string
                let is_last = self.fstring_part_idx >= self.fstring_parts.len()
                    || self.fstring_parts[self.fstring_part_idx..].iter().all(|p| {
                        p.literal.is_empty()
                            && p.expr_text.is_empty()
                            && p.format_spec.is_empty()
                            && p.conversion == 0
                            && !p.has_spec
                    });
                if is_last {
                    if is_tstring {
                        to_push.push((Token::TStringEnd, fb, close_end));
                    } else {
                        to_push.push((Token::FStringEnd, fb, close_end));
                    }
                    self.fstring_quote = None;
                    self.fstring_is_tstring = false;
                }
                // Push to pending stacks in REVERSE (so they come out in correct order)
                for (t, s, e) in to_push.into_iter().rev() {
                    self.fstring_expr_pending.push(t);
                    self.fstring_expr_pos.push(s);
                    self.fstring_expr_end.push(e);
                }
                return self.next_token();
            }
            // Cleanup if we somehow reach here without emitting FStringEnd
            self.fstring_quote = None;
            self.fstring_is_tstring = false;
            self.fstring_parts = Vec::new();
            self.fstring_part_idx = 0;
            self.fstring_expr_pos = Vec::new();
            self.fstring_expr_end = Vec::new();
            return self.next_token(); // Try again with fstring_quote cleared
        }

        if let Some(tok) = self.pending.pop() {
            return tok;
        }

        if self.at_line_start && self.paren_level == 0 {
            self.handle_indent();
            self.at_line_start = false;
            // A lexer-detected indentation error (e.g. unindent does not
            // match) must be surfaced immediately, before the line's first
            // token is consumed.
            if let Some(err) = self.pending_error.take() {
                return Token::LexerError(err);
            }
            if let Some(tok) = self.pending.pop() {
                return tok;
            }
        }

        loop {
            let ch = match self.advance() {
                Some(c) => c,
                None => {
                    // EOF right after an explicit line continuation
                    // (`\` + newline, nothing but trivia after) promises
                    // more input that never comes (mirrors CPython:
                    // `exec('ä = 5\\\n')` is "unexpected EOF while
                    // parsing"). Look back over trailing trivia for the
                    // continuation; strings/comments can't end here
                    // (unterminated ones error first).
                    let mut p = self.pos;
                    while p > 0 && matches!(self.chars.get(p - 1), Some(' ' | '\t' | '\n' | '\r')) {
                        p -= 1;
                    }
                    if p >= 1
                        && self.chars.get(p - 1) == Some(&'\\')
                        && matches!(self.chars.get(p), Some('\n' | '\r'))
                    {
                        return Token::LexerError("unexpected EOF while parsing".to_string());
                    }
                    return self.emit(Token::EndOfFile);
                }
            };
            // Start of the token just scanned (trivia iterations are
            // overwritten by the real token's iteration below), mapped
            // to absolute coordinates at record time (see `tok_start`).
            self.tok_start = self.abs_pos(self.line, self.col.saturating_sub(1).max(1));

            match ch {
                ' ' | '\t' => {
                    continue;
                }
                '#' => {
                    while self.peek() != Some('\n') && self.peek().is_some() {
                        self.advance();
                    }
                    if self.paren_level > 0 {
                        // Inside parentheses/brackets/braces, skip newlines (implicit continuation).
                        // Do NOT set at_line_start here — the logical line hasn't ended (matches
                        // the plain '\n' branch below), otherwise this stale flag survives past the
                        // closing paren and triggers a bogus indent check at the wrong column.
                        if self.peek() == Some('\n') {
                            self.advance(); // consume the newline char
                        }
                        continue;
                    }
                    if self.peek().is_some() {
                        self.at_line_start = true;
                    }
                    return self.emit(Token::Newline);
                }
                '\n' => {
                    if self.paren_level > 0 {
                        continue;
                    }
                    self.at_line_start = true;
                    return self.emit(Token::Newline);
                }
                '\\' => {
                    if self.peek() == Some('\n') || self.peek() == Some('\r') {
                        self.advance();
                        continue;
                    }
                    if self.peek().is_none() {
                        // A line-continuation backslash as the LAST
                        // character of the source (no trailing newline at
                        // all, not even an empty one). In exec mode the
                        // tokenizer's implicit trailing newline turns the
                        // continuation into "unexpected EOF while parsing"
                        // (test_eof.py's test_line_continuation_EOF /
                        // test_eof_with_line_continuation hit this); in
                        // eval mode there is no implicit newline, so it
                        // is "unexpected character after line
                        // continuation" (mirrors CPython: `eval("\\")`
                        // vs `exec("\\")`). Carries an explicit position
                        // (EOF column, or 0 for a lone backslash) so the
                        // offset doesn't fall back to a bogus default.
                        if self.eval_mode {
                            return Token::LexerError(
                                "unexpected character after line continuation".to_string(),
                            );
                        }
                        let (line, col) = self.trailing_backslash_pos();
                        return Token::LexerError(format!(
                            "L{}:{}: unexpected EOF while parsing",
                            line, col
                        ));
                    }
                    // A stray backslash outside strings/continuations is
                    // an error (mirrors CPython: `eval("\\t3")`).
                    return Token::LexerError(
                        "unexpected character after line continuation".to_string(),
                    );
                }
                '\r' => continue,

                // Non-printable characters outside strings are rejected
                // (mirrors CPython: a lone `\xa0` source).
                c if !c.is_ascii() && !Self::is_py_printable(c) => {
                    return Token::LexerError(format!(
                        "invalid non-printable character U+{:04X}",
                        c as u32
                    ));
                }

                // String literals
                '\'' | '"' => {
                    return self.read_string(ch, false, false);
                }
                // Also handle f'...' and f"..." if main loop hits quote directly
                // (after f-prefix detection above)

                // Digits
                '0'..='9' => {
                    return self.read_number(ch);
                }

                // Identifiers and keywords
                c if Self::is_identifier_start(c) => {
                    let mut name = String::new();
                    name.push(c);
                    while self.peek().map_or(false, Self::is_identifier_continue) {
                        name.push(self.advance().unwrap());
                    }
                    // Check for string prefixes: f/F, r/R, b/B, u/U, and the
                    // combined raw variants rf/fr/rb/br (any case) — e.g.
                    // `rf"^www\.|..."` for a raw f-string regex pattern.
                    // Single-char prefixes were previously the only ones
                    // recognized; a combined prefix like "rf" is a valid
                    // identifier-continue sequence, so without this check it
                    // silently fell through to being tokenized as a plain
                    // (undefined) name.
                    let lower_name = name.to_ascii_lowercase();
                    let is_string_prefix = matches!(
                        lower_name.as_str(),
                        "f" | "r" | "b" | "u" | "t" | "fr" | "rf" | "br" | "rb" | "tr" | "rt"
                    );
                    if is_string_prefix && (self.peek() == Some('"') || self.peek() == Some('\'')) {
                        // The string starts at the prefix (`f`, `rf`,
                        // ...): `tok_start` still holds it (only peeks
                        // happened since). Recorded for JoinedStr
                        // spans and synthesized-token fallbacks.
                        self.fstring_start_pos = self.tok_start;
                        let quote = self.advance().unwrap();
                        let raw = lower_name.contains('r');
                        let is_fstring = lower_name.contains('f');
                        let is_tstring = lower_name.contains('t');
                        let is_bytes = lower_name.contains('b');
                        if is_fstring {
                            return self.tokenize_fstring(quote, raw, false);
                        } else if is_tstring {
                            return self.tokenize_fstring(quote, raw, true);
                        } else if is_bytes {
                            return self.read_bytes(quote, raw);
                        } else {
                            return self.read_string(quote, raw, false);
                        }
                    }
                    // A name made solely of string-prefix letters directly
                    // abutting a quote is an invalid prefix (`fu''`,
                    // `fb''`, `urt''`), not a name followed by a string
                    // (mirrors CPython: "prefixes are incompatible").
                    if (self.peek() == Some('"') || self.peek() == Some('\''))
                        && lower_name
                            .chars()
                            .all(|c| matches!(c, 'r' | 'b' | 'u' | 'f' | 't'))
                    {
                        return Token::LexerError("prefixes are incompatible".to_string());
                    }
                    // PEP 3131: identifiers are NFKC-normalized. This handles
                    // compatibility characters (e.g. MICRO SIGN U+00B5 -> GREEK MU
                    // U+03BC, and mathematical fraktur -> ASCII) so that
                    // `µ` and `μ` are the same identifier, and
                    // `𝔘𝔫𝔦𝔠𝔬𝔡𝔢` normalizes to `Unicode`.
                    let normalized: String = name.nfkc().collect();
                    // Record the end span like `emit` does (this
                    // `return match` bypasses it textually).
                    let tok = match normalized.as_str() {
                        "False" => Token::False,
                        "None" => Token::None,
                        "True" => Token::True,
                        "and" => Token::And,
                        "as" => Token::As,
                        "assert" => Token::Assert,
                        "async" => Token::Async,
                        "await" => Token::Await,
                        "break" => Token::Break,
                        "class" => Token::Class,
                        "continue" => Token::Continue,
                        "def" => Token::Def,
                        "del" => Token::Del,
                        "elif" => Token::Elif,
                        "else" => Token::Else,
                        "except" => Token::Except,
                        "finally" => Token::Finally,
                        "for" => Token::For,
                        "from" => Token::From,
                        "global" => Token::Global,
                        "if" => Token::If,
                        "import" => Token::Import,
                        "in" => Token::In,
                        "is" => Token::Is,
                        "lambda" => Token::Lambda,
                        "nonlocal" => Token::Nonlocal,
                        "not" => Token::Not,
                        "or" => Token::Or,
                        "pass" => Token::Pass,
                        "raise" => Token::Raise,
                        "return" => Token::Return,
                        "try" => Token::Try,
                        "while" => Token::While,
                        "with" => Token::With,
                        "yield" => Token::Yield,
                        "match" => Token::Name("match".to_string()),
                        "case" => Token::Name("case".to_string()),
                        "_" => Token::Underscore,
                        _ => Token::Name(normalized),
                    };
                    return self.emit(tok);
                }

                // Operators and delimiters
                '+' => {
                    if self.advance_if('=') {
                        return self.emit(Token::PlusEqual);
                    } else {
                        return self.emit(Token::Plus);
                    }
                }
                '-' => {
                    if self.advance_if('=') {
                        return self.emit(Token::MinusEqual);
                    } else if self.advance_if('>') {
                        return self.emit(Token::Arrow);
                    } else {
                        return self.emit(Token::Minus);
                    }
                }
                '*' => {
                    if self.advance_if('*') {
                        if self.advance_if('=') {
                            return self.emit(Token::DoubleStarEqual);
                        } else {
                            return self.emit(Token::DoubleStar);
                        }
                    } else if self.advance_if('=') {
                        return self.emit(Token::StarEqual);
                    } else {
                        return self.emit(Token::Star);
                    }
                }
                '/' => {
                    if self.advance_if('/') {
                        if self.advance_if('=') {
                            return self.emit(Token::DoubleSlashEqual);
                        } else {
                            return self.emit(Token::DoubleSlash);
                        }
                    } else if self.advance_if('=') {
                        return self.emit(Token::SlashEqual);
                    } else {
                        return self.emit(Token::Slash);
                    }
                }
                '%' => {
                    if self.advance_if('=') {
                        return self.emit(Token::PercentEqual);
                    } else {
                        return self.emit(Token::Percent);
                    }
                }
                '@' => {
                    if self.advance_if('=') {
                        return self.emit(Token::AtEqual);
                    } else {
                        return self.emit(Token::At);
                    }
                }
                '&' => {
                    if self.advance_if('=') {
                        return self.emit(Token::AmpersandEqual);
                    } else {
                        return self.emit(Token::Ampersand);
                    }
                }
                '|' => {
                    if self.advance_if('=') {
                        return self.emit(Token::PipeEqual);
                    } else {
                        return self.emit(Token::Pipe);
                    }
                }
                '^' => {
                    if self.advance_if('=') {
                        return self.emit(Token::CaretEqual);
                    } else {
                        return self.emit(Token::Caret);
                    }
                }
                '~' => {
                    return self.emit(Token::Tilde);
                }
                '<' => {
                    if self.advance_if('<') {
                        if self.advance_if('=') {
                            return self.emit(Token::LeftShiftEqual);
                        } else {
                            return self.emit(Token::LeftShift);
                        }
                    } else if self.advance_if('=') {
                        return self.emit(Token::LessEqual);
                    } else if self.advance_if('>') {
                        // `<>` — legacy not-equal. Only valid with the
                        // barry_as_FLUFL future flag; the PARSER decides
                        // (accepts it then, rejects it otherwise).
                        return self.emit(Token::LessGreater);
                    } else {
                        return self.emit(Token::Less);
                    }
                }
                '>' => {
                    if self.advance_if('>') {
                        if self.advance_if('=') {
                            return self.emit(Token::RightShiftEqual);
                        } else {
                            return self.emit(Token::RightShift);
                        }
                    } else if self.advance_if('=') {
                        return self.emit(Token::GreaterEqual);
                    } else {
                        return self.emit(Token::Greater);
                    }
                }
                '=' => {
                    if self.advance_if('=') {
                        return self.emit(Token::EqualEqual);
                    } else {
                        return self.emit(Token::Equal);
                    }
                }
                '!' => {
                    if self.advance_if('=') {
                        return self.emit(Token::NotEqual);
                    } else {
                        return self.emit(Token::Name("!".to_string()));
                    }
                }
                '(' => {
                    self.paren_level += 1;
                    return self.emit(Token::LeftParen);
                }
                ')' => {
                    if self.paren_level > 0 {
                        self.paren_level -= 1;
                    }
                    return self.emit(Token::RightParen);
                }
                '[' => {
                    self.paren_level += 1;
                    return self.emit(Token::LeftBracket);
                }
                ']' => {
                    if self.paren_level > 0 {
                        self.paren_level -= 1;
                    }
                    return self.emit(Token::RightBracket);
                }
                '{' => {
                    self.paren_level += 1;
                    return self.emit(Token::LeftBrace);
                }
                '}' => {
                    if self.paren_level > 0 {
                        self.paren_level -= 1;
                    }
                    return self.emit(Token::RightBrace);
                }
                ',' => {
                    return self.emit(Token::Comma);
                }
                ':' => {
                    if self.advance_if('=') {
                        return self.emit(Token::Walrus);
                    } else {
                        return self.emit(Token::Colon);
                    }
                }
                ';' => {
                    return self.emit(Token::Semicolon);
                }
                '.' => {
                    if self.peek() == Some('.') && self.peek_ahead(1) == Some('.') {
                        self.advance();
                        self.advance();
                        return self.emit(Token::Ellipsis);
                    } else if self.peek().map_or(false, |c| c.is_ascii_digit()) {
                        // A float literal with no leading digit (`.995`,
                        // `.5e10`) — real, standard Python syntax (real code:
                        // CPython's own test suite uses `.995 * digits`),
                        // previously not recognized at all since this
                        // dispatch only ever produced a bare Dot/Ellipsis
                        // token regardless of what followed.
                        return self.read_number('.');
                    } else {
                        return self.emit(Token::Dot);
                    }
                }

                // `$`, `?` and backtick are ASCII punctuation with
                // no token meaning — CPython's tokenizer passes them
                // through as single-char tokens and the parser reports
                // plain "invalid syntax" (only non-ASCII gets "invalid
                // character ...", e.g. test_syntax's `import ä £`).
                '$' | '?' | '`' => {
                    return Token::LexerError("invalid syntax".to_string());
                }
                _ => {
                    let (line, _) = self.get_line_col();
                    let col = if self.col > 1 { self.col - 1 } else { 1 };
                    return Token::LexerError(format!(
                        "L{}:{}: invalid character '{}' (U+{:04X})",
                        line, col, ch, ch as u32
                    ));
                }
            }
        }
    }

    fn handle_indent(&mut self) {
        let mut indent = 0;
        loop {
            match self.peek() {
                Some(' ') => {
                    indent += 1;
                    self.advance();
                }
                Some('\t') => {
                    indent += 8;
                    self.advance();
                }
                Some('#') => {
                    while self.peek() != Some('\n') && self.peek().is_some() {
                        self.advance();
                    }
                    if self.peek().is_some() {
                        self.advance();
                        self.at_line_start = true;
                    }
                    indent = 0;
                    continue;
                }
                Some('\n') => {
                    self.advance();
                    indent = 0;
                    self.at_line_start = true;
                    continue;
                }
                Some('\r') => {
                    self.advance();
                    continue;
                }
                Some('\\') => {
                    self.advance();
                    if self.peek() == Some('\n') || self.peek() == Some('\r') {
                        self.advance();
                    } else if self.peek().is_none() {
                        // A lone trailing backslash.
                        if self.eval_mode {
                            // CPython eval mode: "unexpected character after line continuation"
                            self.pending_error =
                                Some("unexpected character after line continuation".to_string());
                        } else {
                            // CPython exec mode: "unexpected EOF while parsing"
                            let (line, col) = self.trailing_backslash_pos();
                            self.pending_error =
                                Some(format!("L{}:{}: unexpected EOF while parsing", line, col));
                        }
                        break;
                    } else {
                        // A backslash at indent position not followed by
                        // a newline is an error, not a continuation
                        // (mirrors CPython: `eval("\\t3")`).
                        self.pending_error =
                            Some("unexpected character after line continuation".to_string());
                        break;
                    }
                    indent = 0;
                    continue;
                }
                _ => break,
            }
        }
        if self.peek().is_none() {
            // True EOF: any whitespace counted on this final, never-
            // newline-terminated "line" is NOT a real indented line — real
            // Python's tokenizer treats trailing whitespace before EOF as
            // completely insignificant, regardless of how it compares to
            // the current indent stack. The `indent > current` branch that
            // used to run here (shared with the blank-line case below) was
            // a real bug: a source string ending in trailing whitespace but
            // no final newline (extremely common for anything built via
            // string concatenation/`.format()`/`textwrap.indent`, e.g.
            // `exec()`'d generated code) would spuriously emit a `Token::
            // Indent` for that whitespace, which the parser then choked on
            // ("Expected expression, got Indent") since there was no actual
            // statement there. Confirmed via CPython's own `test_listcomps.
            // py`, whose `_check_in_scopes` helper builds exec'd source via
            // `.format()` that ends in exactly this shape. Fix: at true
            // EOF, unconditionally close every remaining open indent level
            // (down to 0) via Dedent — never push a new Indent.
            while self.indent_stack.len() > 1 {
                self.indent_stack.pop();
                self.pending.insert(0, Token::Dedent);
            }
            return;
        }
        if self.peek() == Some('\n') {
            // Still need to handle indent/dedent for comment-only lines
            // followed by blank lines (the # handler consumes the \n but
            // the indent check is skipped due to early return)
            let current = self.indent_stack.last().copied().unwrap_or(0);
            if indent > current {
                self.indent_stack.push(indent);
                self.pending.push(Token::Indent);
            } else if indent < current {
                while let Some(level) = self.indent_stack.last().cloned() {
                    if level == indent {
                        break;
                    }
                    self.indent_stack.pop();
                    self.pending.insert(0, Token::Dedent);
                }
            }
            return;
        }
        let current = self.indent_stack.last().copied().unwrap_or(0);
        if indent > current {
            self.indent_stack.push(indent);
            self.pending.push(Token::Indent);
        } else if indent < current {
            // `indent` must match some level already on the stack; otherwise
            // it's an "unindent does not match any outer indentation level"
            // error (e.g. dedenting to a column that was never an indent).
            let matches = self.indent_stack.iter().any(|&level| level == indent);
            if !matches {
                self.pending_error =
                    Some("unindent does not match any outer indentation level".to_string());
            } else {
                let mut dedents = Vec::new();
                while let Some(&level) = self.indent_stack.last() {
                    if level == indent {
                        break;
                    }
                    self.indent_stack.pop();
                    dedents.push(Token::Dedent);
                }
                // Push dedents in reverse so that innermost dedent is emitted first
                for d in dedents.into_iter().rev() {
                    self.pending.push(d);
                }
            }
        }
    }

    /// If `buf` ends with a string-prefix word (e.g. the `f` in
    /// `{f"`), return whether it interpolates (`f`/`t` prefixes do;
    /// `r`/`b`/`u` don't). `None` when there is no prefix (plain
    /// string literal, or the quote follows an identifier char —
    /// `{name"x"}` is a syntax error in CPython anyway).
    fn string_prefix_kind(buf: &str) -> Option<bool> {
        let run_len = buf
            .bytes()
            .rev()
            .take_while(|b| b.is_ascii_alphabetic())
            .count();
        if run_len == 0 {
            return None;
        }
        let run = &buf[buf.len() - run_len..];
        // The run must be a standalone prefix, not the tail of an
        // identifier (`{myvar"x"}` isn't a nested string).
        if buf.len() > run_len {
            let prev = buf[..buf.len() - run_len].chars().next_back().unwrap();
            if prev == '_' || prev.is_alphanumeric() {
                return None;
            }
        }
        let lower = run.to_ascii_lowercase();
        if !matches!(
            lower.as_str(),
            "f" | "r" | "b" | "u" | "t" | "fr" | "rf" | "tr" | "rt" | "br" | "rb"
        ) {
            return None;
        }
        Some(lower.contains('f') || lower.contains('t'))
    }

    /// Consume a complete quoted string starting at its opening quote
    /// (`self.peek() == quote`), appending it verbatim to `dest`.
    /// Triple quotes and backslash escapes are honored; for
    /// interpolating prefixes (`f`/`t`), `{...}` fields nest (brace
    /// depth tracked) and strings inside fields recurse through this
    /// same function — enabling PEP 701 same-quote nesting.
    fn skip_nested_string(
        &mut self,
        dest: &mut String,
        quote: char,
        interpolating: bool,
    ) -> Option<Token> {
        // NOTE: the opening quote was already consumed by the caller's
        // `match self.advance()` — `quote` carries it.
        // Error prefix for fields nested inside THIS string (derived
        // from its own prefix run: `t`/`tr`/`rt` → t-string).
        let nprefix: &'static str = {
            let run_len = dest
                .bytes()
                .rev()
                .take_while(|b| b.is_ascii_alphabetic())
                .count();
            let run = &dest[dest.len() - run_len..];
            if run.to_ascii_lowercase().contains('t') {
                "t-string"
            } else {
                "f-string"
            }
        };
        dest.push(quote);
        let triple = self.peek() == Some(quote) && self.peek_ahead(1) == Some(quote);
        if triple {
            dest.push(self.advance().unwrap());
            dest.push(self.advance().unwrap());
        }
        let mut depth = 0u32;
        // Inner-field validation state (mirrors the outer field
        // machine): without it, invalid nested fields like the
        // `F'{:'` in `F'{[F'{:'}[F'{:'}]]]` are swallowed as literal
        // text and misreported downstream ("expecting '}'" instead of
        // "valid expression required before ':'").
        let mut istr_q: Option<char> = None;
        let mut istr_triple = false;
        let mut icontent = false;
        let mut istate: u8 = 0; // 0 = expression, 2 = format spec, 3 = conversion
        let mut iconv = false;
        let mut ispec = false;
        let mut ibrackets = 0u32;
        // Mark expression content (whitespace doesn't count, mirroring
        // `ftexpr_has_content`).
        macro_rules! imark {
            ($c:expr) => {
                if depth >= 1 && istate != 2 && !$c.is_ascii_whitespace() {
                    icontent = true;
                }
            };
        }
        loop {
            match self.advance() {
                None => break,
                Some('\\') => {
                    dest.push('\\');
                    if let Some(n) = self.advance() {
                        dest.push(n);
                        imark!(n);
                    }
                }
                Some(c) if c == '\'' || c == '"' => {
                    if let Some(q) = istr_q {
                        dest.push(c);
                        imark!(c);
                        if c == q {
                            if istr_triple {
                                if self.peek() == Some(q) && self.peek_ahead(1) == Some(q) {
                                    dest.push(self.advance().unwrap());
                                    dest.push(self.advance().unwrap());
                                    istr_q = None;
                                }
                            } else {
                                istr_q = None;
                            }
                        }
                    } else if Self::string_prefix_kind(dest).is_some() {
                        // Nested string with its own prefix — recurse.
                        let inner_interp = Self::string_prefix_kind(dest).unwrap_or(false);
                        if let Some(err) = self.skip_nested_string(dest, c, inner_interp) {
                            return Some(err);
                        }
                        imark!(c);
                    } else if c == quote {
                        if depth == 0 {
                            if triple {
                                // Need all three quotes to close
                                // (`don't` inside triple quotes is
                                // literal, not a closer).
                                if self.peek() == Some(quote) && self.peek_ahead(1) == Some(quote) {
                                    dest.push(c);
                                    dest.push(self.advance().unwrap());
                                    dest.push(self.advance().unwrap());
                                    break;
                                }
                                dest.push(c);
                            } else {
                                dest.push(c);
                                break;
                            }
                        } else {
                            // Same-quote plain string inside a field
                            // (`f"{"k"}"`): skip to its own closer.
                            dest.push(c);
                            self.skip_plain_rest(dest, c);
                            icontent = true;
                        }
                    } else {
                        // A different quote opens a plain string inside
                        // the nested string/field — track it so `:`,
                        // `}` etc. inside don't affect validation.
                        istr_q = Some(c);
                        istr_triple = self.peek() == Some(c) && self.peek_ahead(1) == Some(c);
                        dest.push(c);
                        if istr_triple {
                            dest.push(self.advance().unwrap());
                            dest.push(self.advance().unwrap());
                        }
                        imark!(c);
                    }
                }
                Some('{') if interpolating && istr_q.is_none() => {
                    dest.push('{');
                    depth += 1;
                    if depth == 1 {
                        icontent = false;
                        istate = 0;
                        iconv = false;
                        ispec = false;
                        ibrackets = 0;
                    } else {
                        icontent = true;
                    }
                }
                Some('}') if interpolating && istr_q.is_none() && depth > 0 => {
                    dest.push('}');
                    depth -= 1;
                    if depth == 0 {
                        if !icontent && !iconv && !ispec {
                            return Some(self.lex_err_here(format!(
                                "{}: valid expression required before '}}'",
                                nprefix
                            )));
                        }
                    } else {
                        icontent = true;
                    }
                }
                // Walrus-aware `:` split (mirrors the outer machine):
                // `{x:=1}` keeps `:=`, a bare top-level `:` with an
                // empty expression is an error, otherwise it opens
                // the format spec.
                Some(':')
                    if interpolating
                        && istr_q.is_none()
                        && depth == 1
                        && (istate == 0 || istate == 3)
                        && ibrackets == 0 =>
                {
                    if self.peek() == Some('=') {
                        dest.push(':');
                        dest.push('=');
                        self.advance();
                        icontent = true;
                    } else if istate == 0 && !icontent {
                        return Some(self.lex_err_here(format!(
                            "{}: valid expression required before ':'",
                            nprefix
                        )));
                    } else {
                        istate = 2;
                        ispec = true;
                        dest.push(':');
                    }
                }
                Some('!')
                    if interpolating
                        && istr_q.is_none()
                        && depth == 1
                        && istate == 0
                        && ibrackets == 0 =>
                {
                    if self.peek() == Some('=') {
                        dest.push('!');
                        dest.push('=');
                        self.advance();
                        icontent = true;
                    } else if !icontent {
                        return Some(self.lex_err_here(format!(
                            "{}: valid expression required before '!'",
                            nprefix
                        )));
                    } else {
                        iconv = true;
                        istate = 3;
                        dest.push('!');
                    }
                }
                Some('=')
                    if interpolating
                        && istr_q.is_none()
                        && depth == 1
                        && istate == 0
                        && ibrackets == 0 =>
                {
                    if !icontent {
                        return Some(self.lex_err_here(format!(
                            "{}: valid expression required before '='",
                            nprefix
                        )));
                    }
                    dest.push('=');
                    icontent = true;
                }
                // `#` starts a comment inside an inner field (outside
                // strings/brackets) — comments are not content.
                Some('#')
                    if interpolating
                        && istr_q.is_none()
                        && depth >= 1
                        && istate == 0
                        && ibrackets == 0 =>
                {
                    dest.push('#');
                    while let Some(n) = self.peek() {
                        if n == '\n' {
                            break;
                        }
                        dest.push(self.advance().unwrap());
                    }
                }
                Some(c @ ('(' | '[')) if interpolating && istr_q.is_none() => {
                    dest.push(c);
                    ibrackets += 1;
                    imark!(c);
                }
                Some(c @ (')' | ']')) if interpolating && istr_q.is_none() => {
                    dest.push(c);
                    ibrackets = ibrackets.saturating_sub(1);
                    imark!(c);
                }
                Some(c) => {
                    dest.push(c);
                    imark!(c);
                }
            }
        }
        None
    }

    /// Skip the remainder of a plain (non-interpolating) string whose
    /// opening quote was already consumed, through its matching closer.
    fn skip_plain_rest(&mut self, dest: &mut String, quote: char) {
        let triple = self.peek() == Some(quote) && self.peek_ahead(1) == Some(quote);
        if triple {
            dest.push(self.advance().unwrap());
            dest.push(self.advance().unwrap());
        }
        loop {
            match self.advance() {
                None => break,
                Some('\\') => {
                    dest.push('\\');
                    if let Some(n) = self.advance() {
                        dest.push(n);
                    }
                }
                Some(c) if c == quote => {
                    dest.push(c);
                    if triple {
                        if self.peek() == Some(quote) && self.peek_ahead(1) == Some(quote) {
                            dest.push(self.advance().unwrap());
                            dest.push(self.advance().unwrap());
                            break;
                        }
                    } else {
                        break;
                    }
                }
                Some(c) => dest.push(c),
            }
        }
    }

    /// Approximation of CPython `Py_UNICODE_ISPRINTABLE` for tokenizer
    /// validation. Only false-negatives matter here: wrongly accepting
    /// defers to downstream errors, but wrongly rejecting breaks valid
    /// code (identifiers like `café`, emoji) — so when in doubt, true.
    fn is_py_printable(c: char) -> bool {
        if c == ' ' || c.is_alphabetic() || c.is_numeric() {
            return true;
        }
        if c.is_control() {
            return false;
        }
        match c {
            // Unicode spaces (Zs except U+0020), line/paragraph
            // separators, and notorious format characters.
            '\u{a0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200f}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202f}'
            | '\u{205f}'
            | '\u{3000}'
            | '\u{feff}'
            | '\u{ad}' => false,
            _ => true,
        }
    }

    /// Message prefix for f/t-string tokenizer errors.
    fn ft_prefix(is_tstring: bool) -> &'static str {
        if is_tstring {
            "t-string"
        } else {
            "f-string"
        }
    }

    /// Whether an interpolation expression buffer holds real content:
    /// whitespace and whole `#`-comment lines are skipped, so
    /// `{ # only a comment\n}` counts as empty but `{ # c\n86 }`
    /// counts as content.
    fn ftexpr_has_content(expr: &str) -> bool {
        let mut rest = expr.trim_start_matches(|c: char| c.is_whitespace());
        loop {
            match rest.chars().next() {
                None => return false,
                Some('#') => match rest.find('\n') {
                    Some(i) => rest = rest[i + 1..].trim_start_matches(|c: char| c.is_whitespace()),
                    None => return false,
                },
                _ => return true,
            }
        }
    }

    /// Whether `c` can start an interpolation expression (used after
    /// `{{`: a second `{` not followed by an expression start is an
    /// error). Mirrors CPython's `!annotated_rhs` negative lookahead
    /// conservatively: only characters that can NEVER start an
    /// expression trigger it (anything else flows to the precise
    /// downstream checks).
    fn ft_can_start_expr(c: char, after_dot: Option<char>) -> bool {
        if c.is_alphabetic() || c == '_' {
            return true;
        }
        if c.is_ascii_digit() {
            return true;
        }
        match c {
            '\'' | '"' | '(' | '[' | '{' | '+' | '-' | '~' | '*' => true,
            '.' => matches!(after_dot, Some(d) if d.is_ascii_digit() || d == '.'),
            _ => false,
        }
    }

    /// Whether an f-string field buffer begins with a complete operand,
    /// mirroring PEG `annotated_rhs` prefix matching for `;` errors:
    /// `f'{1+;}'` reports "expecting '=', ...", `f'{+;}'` reports
    /// "after '{'". Leading whitespace/comments are skipped; an empty
    /// buffer, a leading operator/punctuation, a lone `.`, or an
    /// operand-less keyword (`lambda`, `not`, ...) means nothing parsed.
    fn ftexpr_has_operand_start(expr: &str) -> bool {
        let chars: Vec<char> = expr.chars().collect();
        let mut i = 0;
        loop {
            while i < chars.len() && chars[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < chars.len() && chars[i] == '#' {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
                continue;
            }
            break;
        }
        if i >= chars.len() {
            return false;
        }
        let c = chars[i];
        if c.is_ascii_digit() || c == '\'' || c == '"' || c == '(' || c == '[' || c == '{' {
            return true;
        }
        if c == '.' {
            return matches!(chars.get(i + 1), Some(d) if d.is_ascii_digit() || *d == '.');
        }
        if c.is_alphabetic() || c == '_' {
            let mut j = i;
            while j < chars.len() && (chars[j].is_alphanumeric() || chars[j] == '_') {
                j += 1;
            }
            let word: String = chars[i..j].iter().collect();
            return !matches!(
                word.as_str(),
                "lambda"
                    | "not"
                    | "and"
                    | "or"
                    | "await"
                    | "if"
                    | "elif"
                    | "else"
                    | "for"
                    | "while"
                    | "with"
                    | "as"
                    | "def"
                    | "class"
                    | "return"
                    | "import"
                    | "from"
                    | "try"
                    | "except"
                    | "finally"
                    | "raise"
                    | "assert"
                    | "del"
                    | "pass"
                    | "break"
                    | "continue"
                    | "global"
                    | "nonlocal"
                    | "is"
                    | "in"
            );
        }
        false
    }

    /// Strip `#`-comment spans from f-string field text, keeping
    /// everything else verbatim (mirrors CPython's debug-text metadata:
    /// `f'{ # c1\n x # c2\n =}'` renders `' \n x \n =1'`). String
    /// contents are honored (`{"# nooo "=}` keeps its `#`).
    fn strip_ft_comments(text: &str) -> String {
        let chars: Vec<char> = text.chars().collect();
        let mut out = String::new();
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            if c == '\'' || c == '"' {
                let triple = chars.get(i + 1) == Some(&c) && chars.get(i + 2) == Some(&c);
                let n = if triple { 3 } else { 1 };
                for k in 0..n {
                    out.push(chars[i + k]);
                }
                i += n;
                while i < chars.len() {
                    if chars[i] == '\\' {
                        out.push(chars[i]);
                        if i + 1 < chars.len() {
                            out.push(chars[i + 1]);
                        }
                        i += 2;
                        continue;
                    }
                    if chars[i] == c {
                        if triple {
                            if chars.get(i + 1) == Some(&c) && chars.get(i + 2) == Some(&c) {
                                out.push(c);
                                out.push(c);
                                out.push(c);
                                i += 3;
                                break;
                            }
                            out.push(c);
                            i += 1;
                        } else {
                            out.push(c);
                            i += 1;
                            break;
                        }
                    } else {
                        out.push(chars[i]);
                        i += 1;
                    }
                }
                continue;
            }
            if c == '#' {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
                continue;
            }
            out.push(c);
            i += 1;
        }
        out
    }

    /// Whether the text after `{` is a unary operator applied to a bare
    /// `lambda` (`f'{+ lambda:None}'`): unary operands can't be bare
    /// lambdas, so the field has no valid expression (mirrors CPython).
    /// Parenthesized (`+(lambda: x)`) is valid and returns false.
    fn ft_unary_bare_lambda(s: &str) -> bool {
        let mut t = s.trim_start_matches(|c: char| c.is_whitespace());
        if let Some(rest) = t.strip_prefix("not") {
            match rest.chars().next() {
                Some(c) if !c.is_alphanumeric() && c != '_' => t = rest,
                _ => return false,
            }
        } else if let Some(rest) = t.strip_prefix(['+', '-', '~']) {
            t = rest;
        } else {
            return false;
        }
        t = t.trim_start_matches(|c: char| c.is_whitespace());
        if t.starts_with('(') {
            return false;
        }
        Self::ftexpr_is_bare_lambda(t)
    }

    /// Whether the expression is a bare `lambda` (unparenthesized
    /// lambdas are rejected inside interpolations).
    fn ftexpr_is_bare_lambda(expr: &str) -> bool {
        // A `lambda` keyword at nesting depth 0 of the field expression
        // (mirrors CPython's `invalid_expression: 'lambda' ... ':' &
        // FSTRING_MIDDLE`, which fires for `f'{1, lambda:x}'` as well as
        // the prefix case). Strings, comments and bracketed groups hide
        // the keyword (`{'lambda':x}` is a plain string, `(lambda: 1)`
        // is parenthesized and legal).
        let chars: Vec<char> = expr.chars().collect();
        let mut i = 0;
        let mut depth = 0u32;
        while i < chars.len() {
            let c = chars[i];
            if c == '\'' || c == '"' {
                let q = c;
                let triple = chars.get(i + 1) == Some(&q) && chars.get(i + 2) == Some(&q);
                i += if triple { 3 } else { 1 };
                while i < chars.len() {
                    if chars[i] == '\\' {
                        i += 2;
                        continue;
                    }
                    if chars[i] == q {
                        if triple {
                            if chars.get(i + 1) == Some(&q) && chars.get(i + 2) == Some(&q) {
                                i += 3;
                                break;
                            }
                            i += 1;
                        } else {
                            i += 1;
                            break;
                        }
                    } else {
                        i += 1;
                    }
                }
                continue;
            }
            if c == '#' {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
                continue;
            }
            if c == '(' || c == '[' || c == '{' {
                depth += 1;
                i += 1;
                continue;
            }
            if c == ')' || c == ']' || c == '}' {
                depth = depth.saturating_sub(1);
                i += 1;
                continue;
            }
            if depth == 0 && chars[i..].starts_with(&['l', 'a', 'm', 'b', 'd', 'a']) {
                let prev_ok = i == 0 || {
                    let p = chars[i - 1];
                    !p.is_alphanumeric() && p != '_'
                };
                let next_ok = match chars.get(i + 6) {
                    None => true,
                    Some(&n) => !n.is_alphanumeric() && n != '_',
                };
                if prev_ok && next_ok {
                    return true;
                }
            }
            i += 1;
        }
        false
    }

    /// Lexer error prefixed with the current line/col (mirrors the
    /// `L<line>:<col>:` convention `parse_stmt` turns into real
    /// `lineno`/`offset` attributes).
    fn lex_err_here(&self, msg: String) -> Token {
        let (line, col) = self.get_line_col();
        Token::LexerError(format!("L{}:{}: {}", line, col, msg))
    }

    /// Fallback position for synthesized f-string tokens (literals,
    /// conversions, end markers): the f-string's own start.
    fn fstring_fallback_pos(&self) -> (usize, usize) {
        self.fstring_start_pos
    }

    fn tokenize_fstring(&mut self, quote: char, raw: bool, is_tstring: bool) -> Token {
        // Line/col where the f-string starts (unterminated-string errors
        // report HERE, not at EOF — mirrors CPython). NOTE:
        // `fstring_start_pos` is set by the caller (prefix branch)
        // because only it still knows the prefix start.
        let (start_line, start_col) = self.get_line_col();
        // Read the entire f-string, splitting into literal and expression parts
        // Each part: (literal_text, expr_text, format_spec_text, conversion)
        let triple = self.peek() == Some(quote) && self.peek_ahead(1) == Some(quote);
        if triple {
            self.advance();
            self.advance();
        }
        let mut parts: Vec<FStringPartScan> = Vec::new();
        let mut literal = String::new();
        let mut terminated = false;
        let prefix = Self::ft_prefix(is_tstring);
        // Absolute start of the pending literal text (for Constant
        // spans): past the opening quotes; reset past each field
        // close.
        let mut lit_start = self.abs_pos(self.line, self.col);
        loop {
            match self.advance() {
                None => break,
                Some(c) if c == '\\' && raw => {
                    // Raw f-string (rf"...") — a backslash still escapes
                    // exactly the next character for tokenizing purposes
                    // (so \" doesn't end the string), while both chars stay
                    // literal in the content; the escaped char must be
                    // consumed here (see the matching fix/comment in
                    // read_string) or a following quote gets misread as
                    // the terminator. EXCEPT before a brace: `\{{` /
                    // `\}}` (deprecated CPython syntax) means a literal
                    // backslash followed by the `{{`/`}}` doubled-brace
                    // escape, so the brace is left for that handler.
                    literal.push(c);
                    if let Some(next) = self.peek() {
                        if next != '{' && next != '}' {
                            literal.push(self.advance().unwrap());
                        }
                    }
                }
                Some(c) if c == '\\' => {
                    if matches!(self.peek(), Some('{') | Some('}')) {
                        // Deprecated backslash-before-brace: keep the
                        // backslash literal so the following `{{`/`}}`
                        // forms the doubled-brace escape — but warn
                        // (mirrors CPython's SyntaxWarning).
                        self.emit_syntax_warning(
                            "\"\\{\" is an invalid escape sequence. Such sequences will not work in the future. Did you mean \"\\\\{\"? A raw string is also an option.",
                            "SyntaxWarning",
                        );
                        literal.push('\\');
                        continue;
                    }
                    let next = self.advance();
                    match next {
                        Some('N') if self.peek() == Some('{') => {
                            // \N{...} unicode name escape: the '{' must NOT
                            // be treated as an f-string field start.
                            match self.lex_unicode_name_escape(quote, triple) {
                                Ok(ch) => literal.push(ch),
                                Err(tok) => return tok,
                            }
                        }
                        Some('N') => {
                            // `\N` not followed by `{` is malformed.
                            return Self::malformed_n_escape();
                        }
                        Some(c) => {
                            // Backslash-newline is a line continuation
                            // (removed, like the str/bytes lexer does),
                            // not a literal newline. CR and CRLF count
                            // too (mirrors CPython: `f"\\\r"` is `''`).
                            if c == '\n' {
                                continue;
                            }
                            if c == '\r' {
                                if self.peek() == Some('\n') {
                                    self.advance();
                                }
                                continue;
                            }
                            // Full escape decoding (mirrors read_string):
                            // \a \b \f \v \xHH \uHHHH \UHHHHHHHH
                            // \0-\7 octal. Undecoded escapes (e.g. `\d`)
                            // stay literal here; invalid ones in format
                            // specs warn separately (see the `\\`-outside-
                            // strings arm below).
                            match c {
                                'n' => literal.push('\n'),
                                't' => literal.push('\t'),
                                'r' => literal.push('\r'),
                                '\\' => literal.push('\\'),
                                '\'' => literal.push('\''),
                                '"' => literal.push('"'),
                                '{' => literal.push('{'),
                                '}' => literal.push('}'),
                                'a' => literal.push('\x07'),
                                'b' => literal.push('\x08'),
                                'f' => literal.push('\x0C'),
                                'v' => literal.push('\x0B'),
                                'x' => {
                                    let h1 = self.advance().unwrap_or('0');
                                    let h2 = self.advance().unwrap_or('0');
                                    let val = u32::from_str_radix(&format!("{}{}", h1, h2), 16)
                                        .unwrap_or(0);
                                    literal.push(char::from_u32(val).unwrap_or('\u{FFFD}'));
                                }
                                'u' => {
                                    let mut val = 0u32;
                                    for _ in 0..4 {
                                        let h = self.advance().unwrap_or('0');
                                        val = val * 16 + h.to_digit(16).unwrap_or(0);
                                    }
                                    literal.push(char::from_u32(val).unwrap_or('\u{FFFD}'));
                                }
                                'U' => {
                                    let mut val = 0u32;
                                    for _ in 0..8 {
                                        let h = self.advance().unwrap_or('0');
                                        val = val * 16 + h.to_digit(16).unwrap_or(0);
                                    }
                                    literal.push(char::from_u32(val).unwrap_or('\u{FFFD}'));
                                }
                                d if ('0'..='7').contains(&d) => {
                                    let mut val = d.to_digit(8).unwrap();
                                    let mut digits = 1;
                                    while digits < 3 {
                                        match self.peek() {
                                            Some(nc) if ('0'..='7').contains(&nc) => {
                                                val = val * 8 + nc.to_digit(8).unwrap();
                                                self.advance();
                                                digits += 1;
                                            }
                                            _ => break,
                                        }
                                    }
                                    literal.push(char::from_u32(val).unwrap_or('\u{FFFD}'));
                                }
                                c => {
                                    // Any other escape is invalid: keep it
                                    // literal (backslash included:
                                    // `eval(r"f'\g'")` is `'\\g'`) but warn
                                    // (mirrors CPython's SyntaxWarning).
                                    self.emit_syntax_warning(
                                        &format!("\"\\{}\" is an invalid escape sequence. Such sequences will not work in the future. Did you mean \"\\\\{}\"? A raw string is also an option.", c, c),
                                        "SyntaxWarning",
                                    );
                                    literal.push('\\');
                                    literal.push(c);
                                }
                            }
                        }
                        None => literal.push('\\'),
                    }
                }
                Some(c) if c == '{' => {
                    if self.peek() == Some('{') {
                        self.advance();
                        literal.push('{');
                    } else {
                        // A `{` directly followed by `,`, `;` or a
                        // non-unary operator can never start a valid
                        // expression (mirrors CPython: `f'{,}'`). A
                        // doubled `{{` is always a literal-brace escape
                        // here (`f'{{,}}'`, `f'{{}}'` are valid), never
                        // a nested field.
                        // (`)`/ `]` are deliberately absent: a bracket
                        // mismatch there reports "unmatched ')'" first
                        // (mirrors CPython: `f'{)#}'`).
                        if matches!(
                            self.peek(),
                            Some(',' | ';' | '/' | '%' | '<' | '>' | '&' | '|' | '^' | '@')
                        ) {
                            return self.lex_err_here(format!(
                                "{}: expecting a valid expression after '{{'",
                                prefix
                            ));
                        }
                        // `.` starts an expression only as `.5` or `...`
                        // (mirrors CPython: `f'{.}'`).
                        if self.peek() == Some('.') {
                            let after_dot = self.peek_ahead(1);
                            if !matches!(after_dot, Some(d) if d.is_ascii_digit() || d == '.') {
                                return self.lex_err_here(format!(
                                    "{}: expecting a valid expression after '{{'",
                                    prefix
                                ));
                            }
                        }
                        // Unary operator on a bare lambda (`f'{+ lambda:None}'`).
                        {
                            let rest: String = (0..16).filter_map(|n| self.peek_ahead(n)).collect();
                            if Self::ft_unary_bare_lambda(&rest) {
                                return self.lex_err_here(format!(
                                    "{}: expecting a valid expression after '{{'",
                                    prefix
                                ));
                            }
                        }
                        // Start of expression — read until matching '}'.
                        // The `{` was just consumed by the match
                        // scrutinee: its absolute start is one column
                        // back. Content starts here (for expression
                        // positions).
                        let field_open = self.abs_pos(self.line, self.col.saturating_sub(1).max(1));
                        let field_abs = self.abs_pos(self.line, self.col);
                        // Past-`}` position of this field's close,
                        // recorded when depth returns to zero below.
                        let mut field_end: (usize, usize) = (0, 0);
                        let mut depth = 1;
                        let mut expr = String::new();
                        let mut conversion: u8 = 0;
                        // Set after `!r`/`!s`/`!a`; only `:`/`}` may
                        // follow a conversion (mirrors CPython's
                        // `['!' NAME] !(':' | '}')` rule).
                        let mut conv_pending = false;
                        let mut format_spec = String::new();
                        let mut debug: bool = false;
                        let mut debug_trailer = String::new();
                        // Absolute source offset of the `:` opening the
                        // format spec (recorded at the flip below).
                        let mut spec_abs: (usize, usize) = (0, 0);
                        // Set when a `#`-comment runs to EOF (swallowing
                        // closers) — the field's `{` never closes.
                        let mut comment_to_eof = false;
                        let mut state: u8 = 0; // 0=expr, 1=after_conv_marker, 2=format_spec
                        let mut bracket_depth: u32 = 0; // track ()[] nesting
                                                        // Per-level field content: (was-in-spec, buffer len
                                                        // at the opening `{`) — validates nested empty
                                                        // fields (`{10:{ }}`) without confusing them with
                                                        // braces inside strings (which never push here).
                        let mut field_stack: Vec<(bool, usize)> = vec![];
                        // Bracket stack (char, field depth) for mismatch
                        // errors (`{a[4)}`).
                        let mut brackets: Vec<(char, u32)> = vec![];
                        let mut str_char: char = '\0'; // track strings: '\'' or '"' or '\0'
                        let mut triple_str: bool = false; // inside a """ / ''' string
                        while depth > 0 {
                            match self.advance() {
                                Some(c) if str_char != '\0' && c == str_char => {
                                    if state == 2 {
                                        format_spec.push(c);
                                    } else {
                                        expr.push(c);
                                    }
                                    if triple_str {
                                        // A triple-quoted string needs all
                                        // three quotes to close.
                                        if self.peek() == Some(c) && self.peek_ahead(1) == Some(c) {
                                            if state == 2 {
                                                format_spec.push(c);
                                                format_spec.push(c);
                                            } else {
                                                expr.push(c);
                                                expr.push(c);
                                            }
                                            self.advance();
                                            self.advance();
                                            str_char = '\0';
                                            triple_str = false;
                                        }
                                    } else {
                                        str_char = '\0';
                                    }
                                }
                                Some(c) if str_char != '\0' => {
                                    // In a format spec, `}` closes an
                                    // unterminated string AND the spec
                                    // itself (CPython accepts `f"{x:'}"`
                                    // with spec=`'`; a truly stray `}`
                                    // errors downstream as usual) — BUT
                                    // only when the string really is
                                    // unterminated: if the `}` is immediately
                                    // followed by its own quote (`{"}"}`),
                                    // it's string CONTENT of a terminated
                                    // string, pushed literally with tracking
                                    // intact.
                                    if c == '}' && state == 2 && self.peek() != Some(str_char) {
                                        str_char = '\0';
                                        depth -= 1;
                                        continue;
                                    }
                                    if state == 2 {
                                        format_spec.push(c);
                                    } else {
                                        expr.push(c);
                                    }
                                    if c == '\\' {
                                        if let Some(next) = self.advance() {
                                            if state == 2 {
                                                format_spec.push(next);
                                            } else {
                                                expr.push(next);
                                            }
                                        }
                                    }
                                }
                                // PEP 701 (3.12+) allows a `#`-comment
                                // inside an f-string expression's braces,
                                // same as real code inside parentheses
                                // (`f'''{ # comment\n3}'''`). This character-
                                // level scanner didn't know `#` starts a
                                // comment at all, so a quote character
                                // WITHIN the comment text (e.g. an
                                // apostrophe in a contraction, `# it's`)
                                // was misread as opening a real string
                                // literal — desyncing `str_char` tracking
                                // for the rest of the scan and eating the
                                // actual closing `}` (and even the outer
                                // f-string's own closing quotes) as if they
                                // were still "inside a string". Skip
                                // straight to end-of-line (or end of input),
                                // preserving the comment text verbatim in
                                // `expr` — the nested tokenizer
                                // (`tokenize_fstring_expr`) already handles
                                // `#`-comments correctly on its own.
                                Some(c @ '#') if str_char == '\0' && state == 0 => {
                                    expr.push(c);
                                    loop {
                                        match self.peek() {
                                            None => {
                                                // The comment runs to EOF,
                                                // swallowing any would-be
                                                // field/string closers
                                                // (`f'{1#}`) — the `{` never
                                                // closes (mirrors CPython).
                                                comment_to_eof = true;
                                                break;
                                            }
                                            Some('\n') => break,
                                            Some(nc) => {
                                                expr.push(nc);
                                                self.advance();
                                            }
                                        }
                                    }
                                }
                                Some('\\') if str_char == '\0' => {
                                    // A backslash in an EXPRESSION (outside
                                    // strings) is an error — except an
                                    // actual line continuation (`\<newline>`,
                                    // legal in triple-quoted f-strings).
                                    // (Mirrors CPython: `f'{\t3}'` raises
                                    // "unexpected character after line
                                    // continuation". CR/CRLF continuations
                                    // are legal here too.)
                                    if state == 0 && !matches!(self.peek(), Some('\n') | Some('\r'))
                                    {
                                        return Token::LexerError(
                                            "unexpected character after line continuation"
                                                .to_string(),
                                        );
                                    }
                                    // A backslash outside strings escapes
                                    // the next char: an escaped quote must
                                    // NOT open string tracking (else
                                    // `f"{x:\'}"` misread `\'` as opening
                                    // a string, desynced brace depth, and
                                    // ate the real terminator). Applies in
                                    // both expression and format-spec.
                                    // In (non-raw) format specs, recognized
                                    // escapes decode to their chars (mirrors
                                    // CPython: `f'{x:\u2603}'` formats with
                                    // `☃`); raw specs keep everything
                                    // literal (`rf"{X:\xFF}"` is `'\\xFF'`).
                                    // Anything else stays verbatim (with a
                                    // SyntaxWarning, see below).
                                    if state == 2 && !raw {
                                        let decoded = match self.peek() {
                                            Some('\'') => Some('\''),
                                            Some('"') => Some('"'),
                                            Some('\\') => Some('\\'),
                                            Some('n') => Some('\n'),
                                            Some('t') => Some('\t'),
                                            Some('r') => Some('\r'),
                                            Some('a') => Some('\x07'),
                                            Some('b') => Some('\x08'),
                                            Some('f') => Some('\x0C'),
                                            Some('v') => Some('\x0B'),
                                            _ => None,
                                        };
                                        if let Some(ch) = decoded {
                                            self.advance();
                                            format_spec.push(ch);
                                            continue;
                                        }
                                        if let Some(c) = self.peek() {
                                            if c == 'x' || c == 'u' || c == 'U' {
                                                self.advance();
                                                let digits = match c {
                                                    'x' => 2,
                                                    'u' => 4,
                                                    _ => 8,
                                                };
                                                let mut val = 0u32;
                                                for _ in 0..digits {
                                                    let h = self.advance().unwrap_or('0');
                                                    val = val * 16 + h.to_digit(16).unwrap_or(0);
                                                }
                                                format_spec.push(
                                                    char::from_u32(val).unwrap_or('\u{FFFD}'),
                                                );
                                                continue;
                                            }
                                            if ('0'..='7').contains(&c) {
                                                self.advance();
                                                let mut val = c.to_digit(8).unwrap();
                                                let mut n = 1;
                                                while n < 3 {
                                                    match self.peek() {
                                                        Some(nc) if ('0'..='7').contains(&nc) => {
                                                            val = val * 8 + nc.to_digit(8).unwrap();
                                                            self.advance();
                                                            n += 1;
                                                        }
                                                        _ => break,
                                                    }
                                                }
                                                format_spec.push(
                                                    char::from_u32(val).unwrap_or('\u{FFFD}'),
                                                );
                                                continue;
                                            }
                                        }
                                    }
                                    // In (non-raw) format specs, an invalid
                                    // escape keeps both chars literally but
                                    // emits a SyntaxWarning (mirrors
                                    // CPython).
                                    if state == 2 && !raw {
                                        if let Some(next) = self.peek() {
                                            if !matches!(
                                                next,
                                                'n' | 't'
                                                    | 'r'
                                                    | '\\'
                                                    | '\''
                                                    | '"'
                                                    | 'a'
                                                    | 'b'
                                                    | 'f'
                                                    | 'v'
                                                    | 'x'
                                                    | 'u'
                                                    | 'U'
                                                    | 'N'
                                                    | '\n'
                                                    | '{'
                                                    | '}'
                                                    | '0'
                                                    ..='7'
                                            ) {
                                                self.emit_syntax_warning(
                                                    &format!(
                                                        "\"\\{}\" is an invalid escape sequence. Such sequences will not work in the future. Did you mean \"\\\\{}\"? A raw string is also an option.",
                                                        next, next
                                                    ),
                                                    "SyntaxWarning",
                                                );
                                            }
                                        }
                                    }
                                    if state == 2 {
                                        format_spec.push('\\');
                                    } else {
                                        expr.push('\\');
                                    }
                                    if let Some(next) = self.advance() {
                                        if state == 2 {
                                            format_spec.push(next);
                                        } else {
                                            expr.push(next);
                                        }
                                    }
                                }
                                Some(c @ ('\'' | '"'))
                                    if str_char == '\0' && (state == 0 || state == 2) =>
                                {
                                    // A quote matching the outer string's own
                                    // quote with nothing after it (`t'{'`)
                                    // is the outer closer, not a string
                                    // opener — push it literally so EOF
                                    // reports the missing `}` (mirrors
                                    // CPython).
                                    if c == quote && self.peek().is_none() {
                                        if state == 2 {
                                            format_spec.push(c);
                                        } else {
                                            expr.push(c);
                                        }
                                        continue;
                                    }
                                    // PEP 701: a quote directly after a string
                                    // PEP 701: a quote directly after a string
                                    // prefix (`f"`, `t'`, `rf"""`, ...) opens a
                                    // NESTED string, not a string literal —
                                    // same-quote nesting like `f"{f"{x}"}"`
                                    // must be skipped whole, or the open/close
                                    // toggling below desyncs brace depth and
                                    // eats the real terminator.
                                    let has_prefix = {
                                        let buf = if state == 2 { &format_spec } else { &expr };
                                        Self::string_prefix_kind(buf).is_some()
                                    };
                                    if has_prefix {
                                        let kind = {
                                            let buf = if state == 2 { &format_spec } else { &expr };
                                            Self::string_prefix_kind(buf).unwrap()
                                        };
                                        if state == 2 {
                                            if let Some(err) =
                                                self.skip_nested_string(&mut format_spec, c, kind)
                                            {
                                                return err;
                                            }
                                        } else if let Some(err) =
                                            self.skip_nested_string(&mut expr, c, kind)
                                        {
                                            return err;
                                        }
                                        continue;
                                    }
                                    // Detect a triple-quoted string (""" / '''):
                                    // the three quotes must all be consumed
                                    // together so the single-quote closer logic
                                    // above doesn't fire on the first one.
                                    // This must also work inside format_spec
                                    // (state == 2), e.g. `f'{2:{"{"}>10}'`
                                    // where `{"{"}` inside the spec contains
                                    // a quoted "{" that must not affect brace
                                    // depth.
                                    if self.peek() == Some(c) && self.peek_ahead(1) == Some(c) {
                                        str_char = c;
                                        triple_str = true;
                                        if state == 2 {
                                            format_spec.push(c);
                                            format_spec.push(c);
                                            format_spec.push(c);
                                        } else {
                                            expr.push(c);
                                            expr.push(c);
                                            expr.push(c);
                                        }
                                        self.advance();
                                        self.advance();
                                    } else {
                                        str_char = c;
                                        if state == 2 {
                                            format_spec.push(c);
                                        } else {
                                            expr.push(c);
                                        }
                                    }
                                }
                                Some('{') => {
                                    // A nested `{` followed by `,`/`;` can
                                    // never start an expression (`{a:{,}}`,
                                    // `{x:{;}}` are errors). Inside a field
                                    // expression there is no `{{` escape:
                                    // braces always nest (`{ {{}} }` is a
                                    // set with a dict). Inside a FORMAT SPEC
                                    // (`state == 2`), `{{` is not an escape
                                    // either — the second `{` opens a nested
                                    // field that still needs a valid start
                                    // (`f'{3:{{>10}'` errors). EXCEPT an
                                    // exact `{{}}` chunk, which CPython keeps
                                    // as literal spec text (`f'{x:{{}}}'`
                                    // formats with `{}`).
                                    if state == 2 && self.peek() == Some('{') {
                                        if self.peek_ahead(1) == Some('}')
                                            && self.peek_ahead(2) == Some('}')
                                        {
                                            // The scrutinee `{` plus the
                                            // next three chars form the
                                            // `{{}}` chunk (the field's real
                                            // closer follows after).
                                            format_spec.push('{');
                                            for _ in 0..3 {
                                                format_spec.push(self.advance().unwrap());
                                            }
                                            continue;
                                        }
                                        let third = self.peek_ahead(1);
                                        let ok = matches!(third, Some(t) if Self::ft_can_start_expr(t, self.peek_ahead(2)))
                                            || third == Some('}');
                                        if !ok {
                                            return self.lex_err_here(format!(
                                                "{}: expecting a valid expression after '{{'",
                                                prefix
                                            ));
                                        }
                                    }
                                    // A `{` right after a conversion can only
                                    // be `:`/`}` (mirrors CPython:
                                    // `f'{"s"!r{":10"}}'`).
                                    if conv_pending {
                                        return self.lex_err_here(format!(
                                            "{}: expecting ':' or '}}'",
                                            prefix
                                        ));
                                    }
                                    if matches!(
                                        self.peek(),
                                        Some(
                                            ',' | ';'
                                                | '/'
                                                | '%'
                                                | '<'
                                                | '>'
                                                | '&'
                                                | '|'
                                                | '^'
                                                | '@'
                                        )
                                    ) {
                                        return self.lex_err_here(format!(
                                            "{}: expecting a valid expression after '{{'",
                                            prefix
                                        ));
                                    }
                                    // `.` starts an expression only as `.5`
                                    // or `...` (mirrors the main-loop rule).
                                    if self.peek() == Some('.') {
                                        let after_dot = self.peek_ahead(1);
                                        if !matches!(after_dot, Some(d) if d.is_ascii_digit() || d == '.')
                                        {
                                            return self.lex_err_here(format!(
                                                "{}: expecting a valid expression after '{{'",
                                                prefix
                                            ));
                                        }
                                    }
                                    // `depth` must track brace nesting
                                    // uniformly across BOTH the expression
                                    // and the format-spec (`state == 2`)
                                    // portions, not just the expression —
                                    // a nested field inside the spec
                                    // (`f'{value:{bad_format_spec}}'`) also
                                    // opens a brace that its own matching
                                    // `}` must close before the OUTER `}`
                                    // is allowed to end the whole
                                    // `{expr:spec}` construct. Previously
                                    // only incrementing on `state == 0`
                                    // left the spec's nested `{` untracked,
                                    // so its own `}` was consumed as the
                                    // outer terminator instead — truncating
                                    // the captured spec text one character
                                    // short of its real closing brace
                                    // (`test_format.py`'s
                                    // `test_better_error_message_format`:
                                    // the spec came out as `{bad_format_spec`
                                    // instead of evaluating to `%M`).
                                    // Cap nesting depth (mirrors CPython's
                                    // MAX_EXPR_NESTING=3: `f"{1+2:{1+2:{1+1:
                                    // {1}}}}"`, four levels deep, is an
                                    // error, while displays like `{ {{}} }`
                                    // stay under the cap).
                                    if depth + 1 > 3 {
                                        return self.lex_err_here(format!(
                                            "{}: expressions nested too deeply",
                                            prefix
                                        ));
                                    }
                                    depth += 1;
                                    if depth > 1 || state > 0 {
                                        if state == 2 {
                                            format_spec.push('{');
                                        } else {
                                            expr.push('{');
                                        }
                                    }
                                    // Record where this level's content
                                    // starts — AFTER pushing the `{` itself,
                                    // so the nested-empty slice excludes it.
                                    if state == 2 {
                                        field_stack.push((true, format_spec.len()));
                                    } else {
                                        field_stack.push((false, expr.len()));
                                    }
                                }
                                Some('}') => {
                                    // `!` with no conversion character
                                    // (`{x!}`) errors here (this arm
                                    // precedes the state==1 catch-all).
                                    if state == 1 {
                                        return self.lex_err_here(format!(
                                            "{}: missing conversion character",
                                            prefix
                                        ));
                                    }
                                    // A `}` never closes `(`/`[` — report
                                    // the mismatch (mirrors CPython:
                                    // `{a[4}`).
                                    // A `}` only mismatches brackets
                                    // opened at ITS OWN field level —
                                    // outer levels' brackets (e.g. the `(`
                                    // in `{a(b{c}d)}`) are unaffected.
                                    if let Some(&(open, lvl)) = brackets.last() {
                                        if lvl == depth {
                                            return self.lex_err_here(format!(
                                                "{}: closing parenthesis '}}' does not match opening parenthesis '{}'",
                                                prefix, open
                                            ));
                                        }
                                    }
                                    if depth == 1 {
                                        // The field is closing: it must hold
                                        // a real expression, and a bare
                                        // lambda needs parentheses (mirrors
                                        // CPython's t/f-string errors).
                                        if !Self::ftexpr_has_content(&expr) {
                                            return self.lex_err_here(format!(
                                                "{}: valid expression required before '}}'",
                                                prefix
                                            ));
                                        }
                                        if Self::ftexpr_is_bare_lambda(&expr) && bracket_depth == 0
                                        {
                                            return self.lex_err_here(format!(
                                                "{}: lambda expressions are not allowed without parentheses",
                                                prefix
                                            ));
                                        }
                                    } else if let Some((was_spec, start)) = field_stack.pop() {
                                        // A nested field closing inside a
                                        // FORMAT SPEC must hold content
                                        // (`{10:{}}` is an error) — but in
                                        // expression position `{}` is an
                                        // (empty dict) display
                                        // (`{ {{}} }` is valid syntax).
                                        // A leading `:` belongs to the outer
                                        // level, not the content.
                                        if was_spec {
                                            let buf = &format_spec;
                                            let slice = buf.get(start..).unwrap_or("");
                                            let slice = slice.trim_start_matches(|c: char| {
                                                c.is_whitespace() || c == ':'
                                            });
                                            if !Self::ftexpr_has_content(slice) {
                                                return self.lex_err_here(format!(
                                                    "{}: valid expression required before '}}'",
                                                    prefix
                                                ));
                                            }
                                        }
                                    }
                                    if state == 2 && depth > 1 {
                                        format_spec.push('}');
                                        depth -= 1;
                                        conv_pending = false;
                                    } else {
                                        depth -= 1;
                                        conv_pending = false;
                                        if depth == 0 {
                                            // The `}` was just consumed:
                                            // current pos is just past it.
                                            field_end = self.abs_pos(self.line, self.col);
                                        }
                                        if depth > 0 {
                                            if state == 2 {
                                                format_spec.push('}');
                                            } else {
                                                expr.push('}');
                                            }
                                        }
                                    }
                                }
                                Some('=') if depth == 1 && state == 0 => {
                                    // An `=` with no expression before it
                                    // (`{=x}`) is an error.
                                    if !Self::ftexpr_has_content(&expr) {
                                        return self.lex_err_here(format!(
                                            "{}: valid expression required before '='",
                                            prefix
                                        ));
                                    }
                                    // Check if this `=` is a debug `=` (PEP 572)                                    // `f"{1+2 =}"` or `f"{1+2 = # comment\n }"` etc.
                                    // Must look ahead skipping whitespace and comments
                                    // to see if next significant char is `}`/`!`/`:`.
                                    let mut p = self.pos;
                                    // Peek next char after `=` is self.peek()
                                    // already, but we need to look further.
                                    let mut next_idx = p;
                                    // Helper to skip whitespace/newlines
                                    let skip_ws = |idx: &mut usize| {
                                        while *idx < self.chars.len()
                                            && matches!(self.chars[*idx], ' ' | '\t' | '\n' | '\r')
                                        {
                                            *idx += 1;
                                        }
                                    };
                                    skip_ws(&mut next_idx);
                                    // Comments (possibly several) are
                                    // skipped along with following
                                    // whitespace when seeking the next
                                    // significant char.
                                    while next_idx < self.chars.len() && self.chars[next_idx] == '#'
                                    {
                                        while next_idx < self.chars.len()
                                            && self.chars[next_idx] != '\n'
                                        {
                                            next_idx += 1;
                                        }
                                        skip_ws(&mut next_idx);
                                    }
                                    let next_sig = self.chars.get(next_idx).copied();
                                    match self.peek() {
                                        Some('=') => {
                                            // `==` equality, not debug
                                            expr.push('=');
                                            expr.push('=');
                                            self.advance();
                                        }
                                        _ if expr.ends_with(['<', '>']) => {
                                            // `<=` / `>=` comparison, not
                                            // debug and not an error.
                                            expr.push('=');
                                        }
                                        _ if matches!(
                                            next_sig,
                                            Some('}') | Some('!') | Some(':') | None
                                        ) || next_sig == Some(quote) =>
                                        {
                                            debug = true;
                                            // For debug, the `=` and any
                                            // whitespace/comments after it
                                            // are not part of value, but we
                                            // need to consume them so the
                                            // later `#` handling doesn't
                                            // misinterpret. Advance pos to
                                            // next_sig by consuming the
                                            // skipped chars.
                                            // `self.pos` currently points
                                            // just after `=`. Advance to
                                            // next_sig.
                                            let to_consume = next_idx - self.pos;
                                            // The skipped whitespace belongs
                                            // to the debug prefix (`{value = }`
                                            // renders literal `"value = "`),
                                            // but comment TEXT is dropped
                                            // (mirrors CPython's debug_text
                                            // metadata: `f"{1+2 = # c\n  }"`
                                            // renders `'1+2 = \n  3'`).
                                            let mut trailer = String::new();
                                            let mut k = self.pos;
                                            while k < next_idx {
                                                if self.chars[k] == '#' {
                                                    while k < next_idx && self.chars[k] != '\n' {
                                                        k += 1;
                                                    }
                                                } else {
                                                    trailer.push(self.chars[k]);
                                                    k += 1;
                                                }
                                            }
                                            for _ in 0..to_consume {
                                                self.advance();
                                            }
                                            debug_trailer = trailer;
                                        }
                                        _ => {
                                            // `=` that is neither `==` nor a
                                            // debug marker (`{x=y}`) is only
                                            // legal nested in brackets or as
                                            // a lambda default.
                                            if bracket_depth == 0
                                                && !Self::ftexpr_is_bare_lambda(&expr)
                                            {
                                                return self.lex_err_here(format!(
                                                    "{}: expecting '!', or ':', or '}}'",
                                                    prefix
                                                ));
                                            }
                                            expr.push('=');
                                        }
                                    }
                                }
                                Some('!') if depth == 1 && state == 0 => {
                                    if self.peek() == Some('=') {
                                        expr.push('!');
                                        expr.push('=');
                                        self.advance();
                                    } else if !Self::ftexpr_has_content(&expr) {
                                        return self.lex_err_here(format!(
                                            "{}: valid expression required before '!'",
                                            prefix
                                        ));
                                    } else if conversion != 0 {
                                        // A second `!` after a conversion
                                        // (`{x!s!}`) is an error.
                                        return self.lex_err_here(format!(
                                            "{}: expecting ':' or '}}'",
                                            prefix
                                        ));
                                    } else {
                                        state = 1;
                                    }
                                }
                                // Conversion after `!`: read a full NAME
                                // (`f'{3!ss}'` reports `'ss'`, not `'s'`;
                                // mirrors CPython's `fstring_conversion`
                                // + `_PyPegen_check_fstring_conversion`).
                                // Non-NAME followers get the plain message;
                                // whitespace gets its own ("must come right
                                // after"); `}`/`:` mean missing.
                                Some(c) if state == 1 => {
                                    if c.is_alphabetic() || c == '_' {
                                        // `c` was already consumed by the
                                        // match scrutinee; peek/advance only
                                        // for what FOLLOWS it.
                                        let mut name = String::new();
                                        name.push(c);
                                        loop {
                                            match self.peek() {
                                                Some(nc) if nc.is_alphanumeric() || nc == '_' => {
                                                    name.push(nc);
                                                    self.advance();
                                                }
                                                _ => break,
                                            }
                                        }
                                        if name.len() == 1 {
                                            match name.as_str() {
                                                "r" => {
                                                    conversion = 1;
                                                    conv_pending = true;
                                                    state = 0;
                                                    continue;
                                                }
                                                "s" => {
                                                    conversion = 2;
                                                    conv_pending = true;
                                                    state = 0;
                                                    continue;
                                                }
                                                "a" => {
                                                    conversion = 3;
                                                    conv_pending = true;
                                                    state = 0;
                                                    continue;
                                                }
                                                _ => {}
                                            }
                                        }
                                        return self.lex_err_here(format!(
                                            "{}: invalid conversion character '{}': expected 's', 'r', or 'a'",
                                            prefix, name
                                        ));
                                    }
                                    // `!` followed by the string's own
                                    // closing quote (`f'{3!'`) ends the
                                    // field unterminated (mirrors CPython's
                                    // grammar: `'{' annotated_rhs '='?
                                    // ['!' NAME] !'}'`).
                                    if c == quote {
                                        return self
                                            .lex_err_here(format!("{}: expecting '}}'", prefix));
                                    }
                                    if c == '}' || c == ':' {
                                        return self.lex_err_here(format!(
                                            "{}: missing conversion character",
                                            prefix
                                        ));
                                    }
                                    if c.is_whitespace() {
                                        return self.lex_err_here(format!(
                                            "{}: conversion type must come right after the exclamation mark",
                                            prefix
                                        ));
                                    }
                                    return self.lex_err_here(format!(
                                        "{}: invalid conversion character",
                                        prefix
                                    ));
                                }
                                Some(':') if depth == 1 && state == 0 && bracket_depth == 0 => {
                                    if !Self::ftexpr_has_content(&expr) {
                                        return self.lex_err_here(format!(
                                            "{}: valid expression required before ':'",
                                            prefix
                                        ));
                                    }
                                    // A top-level `:=` always opens the
                                    // format spec (`f'{x:=10}'` formats `x`
                                    // with spec `=10`); walrus needs parens
                                    // (`f'{(x:=10)}'`). Bracketed `:=`
                                    // (`f'{a[x:=1]}'`) never reaches here.
                                    // The `:` was just consumed: record its
                                    // absolute start for the spec token.
                                    spec_abs =
                                        self.abs_pos(self.line, self.col.saturating_sub(1).max(1));
                                    state = 2;
                                    conv_pending = false;
                                }
                                // `;` can never appear in an interpolation
                                // outside brackets. Which message mirrors
                                // `invalid_fstring_replacement_field`: with
                                // a complete expression before it (`{1;}`)
                                // the `;` is unexpected ("expecting ...");
                                // otherwise nothing parsed ("after '{'",
                                // e.g. `{+;}`).
                                Some(';')
                                    if str_char == '\0'
                                        && depth == 1
                                        && state == 0
                                        && bracket_depth == 0 =>
                                {
                                    if Self::ftexpr_has_operand_start(&expr) {
                                        return self.lex_err_here(format!(
                                            "{}: expecting '=', or '!', or ':', or '}}'",
                                            prefix
                                        ));
                                    }
                                    return self.lex_err_here(format!(
                                        "{}: expecting a valid expression after '{{'",
                                        prefix
                                    ));
                                }
                                // Raw newlines are not allowed in format
                                // specs of single-quoted f-strings (only
                                // inside nested fields, which run at depth
                                // > 1). Triple-quoted f-strings allow them
                                // (mirrors CPython: `f'''{1:d\n}'''`).
                                Some(c)
                                    if c == '\n'
                                        && str_char == '\0'
                                        && state == 2
                                        && depth == 1
                                        && !triple =>
                                {
                                    return self.lex_err_here(format!(
                                        "{}: newlines are not allowed in format specifiers",
                                        prefix
                                    ));
                                }
                                Some(c) => {
                                    // Non-printable characters are rejected
                                    // (mirrors CPython: `f'''{\xa0}'''`).
                                    // ASCII whitespace is structural (tabs
                                    // and newlines legitimately appear in
                                    // expressions); strings are handled by
                                    // their own arms above, never here.
                                    if !c.is_ascii_whitespace() && !Self::is_py_printable(c) {
                                        return self.lex_err_here(format!(
                                            "invalid non-printable character U+{:04X}",
                                            c as u32
                                        ));
                                    }
                                    // `$`, `?` and backtick can never appear
                                    // in an expression (mirrors CPython's
                                    // grammar errors for `f'{a $ b}'`).
                                    // Specs (state 2) allow arbitrary text.
                                    if state == 0 && matches!(c, '$' | '?' | '`') {
                                        if Self::ftexpr_has_content(&expr) {
                                            return self.lex_err_here(format!(
                                                "{}: expecting '=', or '!', or ':', or '}}'",
                                                prefix
                                            ));
                                        }
                                        return self.lex_err_here(format!(
                                            "{}: expecting a valid expression after '{{'",
                                            prefix
                                        ));
                                    }
                                    if state == 2 {
                                        format_spec.push(c);
                                    } else {
                                        expr.push(c);
                                        // Track bracket nesting for format spec : detection
                                        // (plus a bracket stack for mismatch
                                        // errors like `{a[4)}`).
                                        if c == '(' || c == '[' {
                                            bracket_depth += 1;
                                            brackets.push((c, depth));
                                        } else if c == ')' || c == ']' {
                                            let want = if c == ')' { '(' } else { '[' };
                                            match brackets.last() {
                                                Some(&(open, _)) if open == want => {
                                                    brackets.pop();
                                                }
                                                Some(&(open, _)) => {
                                                    return self.lex_err_here(format!(
                                                        "{}: closing parenthesis '{}' does not match opening parenthesis '{}'",
                                                        prefix, c, open
                                                    ));
                                                }
                                                None => {
                                                    return self.lex_err_here(format!(
                                                        "{}: unmatched '{}'",
                                                        prefix, c
                                                    ));
                                                }
                                            }
                                            if bracket_depth > 0 {
                                                bracket_depth -= 1;
                                            }
                                        }
                                    }
                                }
                                None => break,
                            }
                        }
                        if depth > 0 {
                            // EOF inside an interpolation field.
                            // A `#`-comment that ran to EOF swallowed
                            // every closer — the `{` never closed.
                            if comment_to_eof {
                                return Token::LexerError("'{' was never closed".to_string());
                            }
                            // An unterminated STRING inside the field
                            // reports as such (mirrors CPython:
                            // `f'{"x'`) — UNLESS it uses the f-string's
                            // own quote (`f'{3' f"}"`): then the field
                            // itself is reported unclosed ("'{' was never
                            // closed" when empty, "expecting '}'"
                            // otherwise) instead.
                            if str_char != '\0' && str_char != quote {
                                return self.lex_err_here(format!(
                                    "unterminated string literal (detected at line {})",
                                    self.line
                                ));
                            }
                            if state == 2 {
                                // An invalid expression before the `:` takes
                                // precedence (mirrors CPython's streaming
                                // parse: `f"{1 1:{f"1:..."` reports the
                                // expression's "invalid syntax", not the
                                // unterminated spec).
                                if !expr.trim().is_empty() {
                                    let mut sub = crate::parser::Parser::new(&expr, false);
                                    if let Err(msg) = sub.parse_single_expr() {
                                        return Token::LexerError(msg);
                                    }
                                }
                                return self.lex_err_here(format!(
                                    "{}: expecting '}}', or format specs",
                                    prefix
                                ));
                            }
                            if !Self::ftexpr_has_content(&expr)
                                && conversion == 0
                                && format_spec.is_empty()
                            {
                                return self.lex_err_here("'{' was never closed".to_string());
                            }
                            return self.lex_err_here(format!("{}: expecting '}}'", prefix));
                        }
                        // If debug mode, prepend "{expr}=" (plus any
                        // whitespace skipped after the `=`) to the literal
                        if debug {
                            let dbg_prefix =
                                format!("{}={}", Self::strip_ft_comments(&expr), debug_trailer);
                            literal.push_str(&dbg_prefix);
                            // `!r` is the debug default ONLY without an
                            // explicit conversion or format spec (mirrors
                            // CPython `_get_interpolation_conversion`,
                            // shared by f-strings and t-strings:
                            // `f"{x=:>10}"` has conversion None). A bare
                            // `:` counts as a spec even when empty
                            // (`f'{C()=:}'` calls `__format__('')`).
                            if conversion == 0 && state != 2 {
                                conversion = 1; // Default to !r (repr) for debug
                            }
                        }
                        // `state == 2` records an explicit `:` (even with
                        // an empty spec: `f'{a:}'` has a spec, `f'{a}'`
                        // does not).
                        let has_spec = state == 2;
                        parts.push(FStringPartScan {
                            literal: std::mem::take(&mut literal),
                            expr_text: expr,
                            format_spec,
                            conversion,
                            has_spec,
                            lit_start,
                            field_open,
                            field_start: field_abs,
                            spec_start: spec_abs,
                            field_end,
                        });
                        // The next literal (if any) starts past `}`.
                        lit_start = self.abs_pos(self.line, self.col);
                    }
                }
                Some(c) if c == '}' => {
                    if self.peek() == Some('}') {
                        self.advance();
                        literal.push('}');
                    } else {
                        // A lone `}` outside an expression is an error
                        // (mirrors CPython).
                        return self
                            .lex_err_here(format!("{}: single '}}' is not allowed", prefix));
                    }
                }
                Some(c) if c == quote => {
                    if triple {
                        if self.peek() == Some(quote) && self.peek_ahead(1) == Some(quote) {
                            self.advance();
                            self.advance();
                            terminated = true;
                            self.fstring_close_end = self.abs_pos(self.line, self.col);
                            break;
                        }
                        literal.push(c);
                    } else {
                        terminated = true;
                        self.fstring_close_end = self.abs_pos(self.line, self.col);
                        break;
                    }
                }
                Some(c) => literal.push(c),
            }
        }
        if !terminated {
            let at_start =
                |msg: String| Token::LexerError(format!("L{}:{}: {}", start_line, start_col, msg));
            if is_tstring && triple {
                return at_start("unterminated triple-quoted t-string literal".to_string());
            } else if is_tstring {
                return at_start("unterminated t-string literal".to_string());
            } else if triple {
                return at_start("unterminated triple-quoted f-string literal".to_string());
            } else {
                return at_start("unterminated f-string literal".to_string());
            }
        }
        parts.push(FStringPartScan {
            literal,
            expr_text: String::new(),
            format_spec: String::new(),
            conversion: 0,
            has_spec: false,
            lit_start,
            field_open: (0, 0),
            field_start: (0, 0),
            spec_start: (0, 0),
            field_end: (0, 0),
        });

        self.fstring_quote = Some(quote);
        self.fstring_parts = parts;
        self.fstring_part_idx = 0;
        self.fstring_expr_pending = Vec::new();
        self.fstring_expr_pos = Vec::new();
        self.fstring_expr_end = Vec::new();
        self.fstring_is_tstring = is_tstring;

        // Record the string's own start (the prefix position) as this
        // token's start: start tokens otherwise leave `tok_start` stale
        // (the whole string was just scanned), which would misposition
        // nested f-strings. The end stays stale deliberately —
        // past-the-whole-string would mislead node ends; the string's
        // own end comes from `fstring_close_end` via the End token.
        self.tok_start = self.fstring_start_pos;
        if is_tstring {
            Token::TStringStart
        } else {
            Token::FStringStart
        }
    }

    fn tokenize_fstring_expr(
        &self,
        text: &str,
        base: (usize, usize),
    ) -> (Vec<Token>, Vec<(usize, usize)>, Vec<(usize, usize)>) {
        // Strip leading/trailing whitespace — expressions inside f-strings
        // are not subject to indentation processing. The base offset is
        // advanced past trimmed leading whitespace so recorded positions
        // stay absolute.
        let trimmed = text.trim();
        let lead = text.chars().count() - text.trim_start().chars().count();
        let (mut bline, mut bcol) = base;
        for c in text.chars().take(lead) {
            if c == '\n' {
                bline += 1;
                bcol = 1;
            } else {
                bcol += 1;
            }
        }
        // Tokenize an f-string expression text
        let mut lex = Lexer::new(trimmed, false);
        lex.at_line_start = false;
        lex.base_line = bline;
        lex.base_col = bcol;
        let mut tokens = Vec::new();
        let mut positions = Vec::new();
        let mut ends = Vec::new();
        loop {
            let tok = lex.next_token();
            if tok == Token::EndOfFile {
                break;
            }
            if tok == Token::Newline || tok == Token::Indent || tok == Token::Dedent {
                continue;
            }
            positions.push(lex.tok_start());
            ends.push(lex.tok_end());
            tokens.push(tok);
        }
        (tokens, positions, ends)
    }

    pub fn get_line_col(&self) -> (usize, usize) {
        (self.line, self.col)
    }
}
