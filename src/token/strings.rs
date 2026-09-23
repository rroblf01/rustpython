use super::unicode_name_to_char;
use super::{Lexer, Token};
use crate::object::pystring::PyStr;

/// Encode a Unicode codepoint (including surrogates) as WTF-8 bytes.
fn push_wtf8_codepoint(out: &mut Vec<u8>, cp: u32) {
    if cp < 0x80 {
        out.push(cp as u8);
    } else if cp < 0x800 {
        out.push(0xC0 | ((cp >> 6) as u8));
        out.push(0x80 | ((cp & 0x3F) as u8));
    } else if cp < 0x10000 {
        out.push(0xE0 | ((cp >> 12) as u8));
        out.push(0x80 | ((cp >> 6) as u8 & 0x3F));
        out.push(0x80 | ((cp & 0x3F) as u8));
    } else {
        out.push(0xF0 | ((cp >> 18) as u8));
        out.push(0x80 | ((cp >> 12) as u8 & 0x3F));
        out.push(0x80 | ((cp >> 6) as u8 & 0x3F));
        out.push(0x80 | ((cp & 0x3F) as u8));
    }
}

impl Lexer {
    /// Consume a `\N{...}` escape (called right after `\N` when the next
    /// char is `{`). Returns the decoded char, or a lexer error for a
    /// malformed escape (no closing `}` before the quote/newline/EOF) —
    /// mirrors CPython: `(unicode error) 'unicodeescape' codec can't
    /// decode bytes ...: malformed \N character escape`. Unknown names
    /// stay lenient (U+FFFD), as before.
    pub(crate) fn lex_unicode_name_escape(
        &mut self,
        quote: char,
        triple: bool,
    ) -> Result<char, Token> {
        debug_assert_eq!(self.peek(), Some('{'));
        self.advance(); // consume '{'
        let malformed = || {
            Token::LexerError(
                "(unicode error) 'unicodeescape' codec can't decode bytes in position 0-2: malformed \\N character escape"
                    .to_string(),
            )
        };
        let mut name = String::new();
        loop {
            match self.advance() {
                Some('}') => break,
                None => return Err(malformed()),
                Some('\n') => return Err(malformed()),
                Some(c) if c == quote && !triple => return Err(malformed()),
                Some(c) => name.push(c),
            }
        }
        Ok(unicode_name_to_char(&name)
            // Small lexer table misses most names (e.g. 'LATIN CAPITAL
            // LETTER A WITH DIAERESIS' → U+FFFD, breaking `test_re.py`'s
            // external cases) — fall back to the full Unicode 16.0 table
            // (same one the `re` module uses).
            .or_else(|| crate::unicode_names::unicode_name_lookup(&name))
            .unwrap_or('\u{FFFD}'))
    }

    /// Error token for `\N` NOT followed by `{` (mirrors CPython's
    /// malformed-escape `SyntaxError`, not a literal backslash-N).
    pub(crate) fn malformed_n_escape() -> Token {
        Token::LexerError(
            "(unicode error) 'unicodeescape' codec can't decode bytes in position 0-1: malformed \\N character escape"
                .to_string(),
        )
    }

    pub(crate) fn read_bytes(&mut self, quote: char, raw: bool) -> Token {
        let mut bytes = Vec::new();
        let triple = self.peek() == Some(quote) && self.peek_ahead(1) == Some(quote);
        let mut terminated = false;

        if triple {
            self.advance();
            self.advance();
            loop {
                match self.advance() {
                    None => break,
                    Some(c) if c == '\\' && !raw => {
                        let next = self.advance();
                        match next {
                            Some('n') => bytes.push(b'\n'),
                            Some('t') => bytes.push(b'\t'),
                            Some('r') => bytes.push(b'\r'),
                            Some('\\') => bytes.push(b'\\'),
                            Some('\'') => bytes.push(b'\''),
                            Some('"') => bytes.push(b'"'),
                            Some(d) if ('0'..='7').contains(&d) => {
                                let mut value = d.to_digit(8).unwrap();
                                let mut digits = 1;
                                while digits < 3 {
                                    match self.peek() {
                                        Some(nc) if ('0'..='7').contains(&nc) => {
                                            value = value * 8 + nc.to_digit(8).unwrap();
                                            self.advance();
                                            digits += 1;
                                        }
                                        _ => break,
                                    }
                                }
                                bytes.push(value as u8);
                            }
                            Some('a') => bytes.push(0x07),
                            Some('b') => bytes.push(0x08),
                            Some('f') => bytes.push(0x0c),
                            Some('v') => bytes.push(0x0b),
                            Some('x') => {
                                let h1 = self.advance().unwrap_or('0');
                                let h2 = self.advance().unwrap_or('0');
                                let val =
                                    u8::from_str_radix(&format!("{}{}", h1, h2), 16).unwrap_or(0);
                                bytes.push(val);
                            }
                            Some(c) if c == '\n' => {}
                            Some(c) if c == '\r' => {
                                // `\<CR>` / `\<CRLF>` are line
                                // continuations too (mirrors CPython).
                                if self.peek() == Some('\n') {
                                    self.advance();
                                }
                            }
                            Some(c) => {
                                bytes.push(b'\\');
                                bytes.push(c as u8);
                            }
                            None => bytes.push(b'\\'),
                        }
                    }
                    Some(c) if c == '\\' && raw => {
                        bytes.push(c as u8);
                        if let Some(next) = self.advance() {
                            bytes.push(next as u8);
                        }
                    }
                    Some(c) if c == quote => {
                        if self.peek() == Some(quote) && self.peek_ahead(1) == Some(quote) {
                            self.advance();
                            self.advance();
                            terminated = true;
                            break;
                        }
                        bytes.push(c as u8);
                    }
                    Some(c) => {
                        bytes.push(c as u8);
                    }
                }
            }
            if !terminated {
                return Token::LexerError(format!(
                    "unterminated bytes literal (detected at line {})",
                    self.line - 1
                ));
            }
        } else {
            loop {
                match self.advance() {
                    None => break,
                    Some(c) if c == '\\' && !raw => {
                        let next = self.advance();
                        match next {
                            Some('n') => bytes.push(b'\n'),
                            Some('t') => bytes.push(b'\t'),
                            Some('r') => bytes.push(b'\r'),
                            Some('\\') => bytes.push(b'\\'),
                            Some('\'') => bytes.push(b'\''),
                            Some('"') => bytes.push(b'"'),
                            Some(d) if ('0'..='7').contains(&d) => {
                                let mut value = d.to_digit(8).unwrap();
                                let mut digits = 1;
                                while digits < 3 {
                                    match self.peek() {
                                        Some(nc) if ('0'..='7').contains(&nc) => {
                                            value = value * 8 + nc.to_digit(8).unwrap();
                                            self.advance();
                                            digits += 1;
                                        }
                                        _ => break,
                                    }
                                }
                                bytes.push(value as u8);
                            }
                            Some('a') => bytes.push(0x07),
                            Some('b') => bytes.push(0x08),
                            Some('f') => bytes.push(0x0c),
                            Some('v') => bytes.push(0x0b),
                            Some('x') => {
                                let h1 = self.advance().unwrap_or('0');
                                let h2 = self.advance().unwrap_or('0');
                                let val =
                                    u8::from_str_radix(&format!("{}{}", h1, h2), 16).unwrap_or(0);
                                bytes.push(val);
                            }
                            Some(c) if c == '\n' => {}
                            Some(c) if c == '\r' => {
                                // `\<CR>` / `\<CRLF>` are line
                                // continuations too (mirrors CPython).
                                if self.peek() == Some('\n') {
                                    self.advance();
                                }
                            }
                            Some(c) => {
                                bytes.push(b'\\');
                                bytes.push(c as u8);
                            }
                            None => bytes.push(b'\\'),
                        }
                    }
                    Some(c) if c == '\\' && raw => {
                        bytes.push(c as u8);
                        if let Some(next) = self.advance() {
                            bytes.push(next as u8);
                        }
                    }
                    Some(c) if c == quote => {
                        terminated = true;
                        break;
                    }
                    Some(c) => {
                        bytes.push(c as u8);
                    }
                }
            }
            if !terminated {
                return Token::LexerError(format!(
                    "unterminated string literal (detected at line {})",
                    self.line
                ));
            }
        }

        self.emit(Token::Bytes(bytes))
    }

    pub(crate) fn read_string(&mut self, quote: char, raw: bool, fstring: bool) -> Token {
        // Start position = the opening quote (already consumed by the
        // caller, so one column back) — unterminated-string errors
        // report HERE (mirrors CPython: offset points at the opener).
        let (start_line, start_col) = (self.line, self.col.saturating_sub(1));
        let mut w: Vec<u8> = Vec::new(); // WTF-8 byte buffer
        let triple = self.peek() == Some(quote) && self.peek_ahead(1) == Some(quote);
        let mut terminated = false;
        let mut invalid_escapes: Vec<char> = Vec::new();

        if triple {
            self.advance();
            self.advance();
            loop {
                match self.advance() {
                    None => break,
                    Some(c) if c == '\\' && !raw => {
                        let next = self.advance();
                        match next {
                            Some('n') => w.push(b'\n'),
                            Some('t') => w.push(b'\t'),
                            Some('r') => w.push(b'\r'),
                            Some('\\') => w.push(b'\\'),
                            Some('\'') => w.push(b'\''),
                            Some('"') => w.push(b'"'),
                            Some(d) if ('0'..='7').contains(&d) => {
                                let mut value = d.to_digit(8).unwrap();
                                let mut digits = 1;
                                while digits < 3 {
                                    match self.peek() {
                                        Some(nc) if ('0'..='7').contains(&nc) => {
                                            value = value * 8 + nc.to_digit(8).unwrap();
                                            self.advance();
                                            digits += 1;
                                        }
                                        _ => break,
                                    }
                                }
                                push_wtf8_codepoint(&mut w, value);
                            }
                            Some('a') => w.push(0x07),
                            Some('b') => w.push(0x08),
                            Some('f') => w.push(0x0c),
                            Some('v') => w.push(0x0b),
                            Some('x') => {
                                let h1 = self.advance().unwrap_or('0');
                                let h2 = self.advance().unwrap_or('0');
                                let val =
                                    u8::from_str_radix(&format!("{}{}", h1, h2), 16).unwrap_or(0);
                                push_wtf8_codepoint(&mut w, val as u32);
                            }
                            Some('u') => {
                                let digits: String =
                                    (0..4).map(|_| self.advance().unwrap_or('0')).collect();
                                let val = u32::from_str_radix(&digits, 16).unwrap_or(0xFFFD);
                                push_wtf8_codepoint(&mut w, val);
                            }
                            Some('U') => {
                                let digits: String =
                                    (0..8).map(|_| self.advance().unwrap_or('0')).collect();
                                let val = u32::from_str_radix(&digits, 16).unwrap_or(0xFFFD);
                                push_wtf8_codepoint(&mut w, val);
                            }
                            Some('N') => {
                                if self.peek() == Some('{') {
                                    match self.lex_unicode_name_escape(quote, triple) {
                                        Ok(ch) => push_wtf8_codepoint(&mut w, ch as u32),
                                        Err(tok) => return tok,
                                    }
                                } else {
                                    return Self::malformed_n_escape();
                                }
                            }
                            Some(c) if c == '\n' => {}
                            Some(c) if c == '\r' => {
                                // `\<CR>` / `\<CRLF>` are line
                                // continuations too (mirrors CPython).
                                if self.peek() == Some('\n') {
                                    self.advance();
                                }
                            }
                            Some(c) => {
                                invalid_escapes.push(c);
                                w.push(b'\\');
                                // Invalid escapes keep the char literally
                                // (with a SyntaxWarning) — as full UTF-8,
                                // not `as u8`-truncated: `"\П"` is
                                // backslash + U+041F (PEP 3120).
                                push_wtf8_codepoint(&mut w, c as u32);
                            }
                            None => w.push(b'\\'),
                        }
                    }
                    Some(c) if c == '\\' && raw => {
                        w.push(c as u8);
                        if let Some(next) = self.advance() {
                            push_wtf8_codepoint(&mut w, next as u32);
                        }
                    }
                    Some(c) if c == quote => {
                        if self.peek() == Some(quote) && self.peek_ahead(1) == Some(quote) {
                            self.advance();
                            self.advance();
                            terminated = true;
                            break;
                        }
                        push_wtf8_codepoint(&mut w, c as u32);
                    }
                    Some(c) => {
                        push_wtf8_codepoint(&mut w, c as u32);
                    }
                }
            }
            if !terminated {
                return Token::LexerError(format!(
                    "L{}:{}: unterminated triple-quoted string literal (detected at line {})",
                    start_line, start_col, self.line
                ));
            }
        } else {
            loop {
                match self.advance() {
                    None => break,
                    Some(c) if c == '\\' && !raw => {
                        let next = self.advance();
                        match next {
                            Some('n') => w.push(b'\n'),
                            Some('t') => w.push(b'\t'),
                            Some('r') => w.push(b'\r'),
                            Some('\\') => w.push(b'\\'),
                            Some('\'') => w.push(b'\''),
                            Some('"') => w.push(b'"'),
                            Some(d) if ('0'..='7').contains(&d) => {
                                let mut value = d.to_digit(8).unwrap();
                                let mut digits = 1;
                                while digits < 3 {
                                    match self.peek() {
                                        Some(nc) if ('0'..='7').contains(&nc) => {
                                            value = value * 8 + nc.to_digit(8).unwrap();
                                            self.advance();
                                            digits += 1;
                                        }
                                        _ => break,
                                    }
                                }
                                push_wtf8_codepoint(&mut w, value);
                            }
                            Some('a') => w.push(0x07),
                            Some('b') => w.push(0x08),
                            Some('f') => w.push(0x0c),
                            Some('v') => w.push(0x0b),
                            Some('x') => {
                                let h1 = self.advance().unwrap_or('0');
                                let h2 = self.advance().unwrap_or('0');
                                let val =
                                    u8::from_str_radix(&format!("{}{}", h1, h2), 16).unwrap_or(0);
                                push_wtf8_codepoint(&mut w, val as u32);
                            }
                            Some('u') => {
                                let digits: String =
                                    (0..4).map(|_| self.advance().unwrap_or('0')).collect();
                                let val = u32::from_str_radix(&digits, 16).unwrap_or(0xFFFD);
                                push_wtf8_codepoint(&mut w, val);
                            }
                            Some('U') => {
                                let digits: String =
                                    (0..8).map(|_| self.advance().unwrap_or('0')).collect();
                                let val = u32::from_str_radix(&digits, 16).unwrap_or(0xFFFD);
                                push_wtf8_codepoint(&mut w, val);
                            }
                            Some('N') => {
                                if self.peek() == Some('{') {
                                    match self.lex_unicode_name_escape(quote, triple) {
                                        Ok(ch) => push_wtf8_codepoint(&mut w, ch as u32),
                                        Err(tok) => return tok,
                                    }
                                } else {
                                    return Self::malformed_n_escape();
                                }
                            }
                            Some(c) if c == '\n' => {}
                            Some(c) if c == '\r' => {
                                // `\<CR>` / `\<CRLF>` are line
                                // continuations too (mirrors CPython).
                                if self.peek() == Some('\n') {
                                    self.advance();
                                }
                            }
                            Some(c) => {
                                invalid_escapes.push(c);
                                w.push(b'\\');
                                // Invalid escapes keep the char literally
                                // (with a SyntaxWarning) — as full UTF-8,
                                // not `as u8`-truncated: `"\П"` is
                                // backslash + U+041F (PEP 3120).
                                push_wtf8_codepoint(&mut w, c as u32);
                            }
                            None => w.push(b'\\'),
                        }
                    }
                    Some(c) if c == '\\' && raw => {
                        w.push(c as u8);
                        if let Some(next) = self.advance() {
                            push_wtf8_codepoint(&mut w, next as u32);
                        }
                    }
                    Some(c) if c == '{' && fstring => {
                        if self.peek() == Some('{') {
                            w.push(b'{');
                            self.advance();
                        } else {
                            w.extend_from_slice(b"{...}");
                            let mut depth = 1;
                            while depth > 0 {
                                match self.advance() {
                                    Some('{') => depth += 1,
                                    Some('}') => depth -= 1,
                                    None => break,
                                    _ => {}
                                }
                            }
                        }
                    }
                    Some(c) if c == '}' && fstring => {
                        if self.peek() == Some('}') {
                            w.push(b'}');
                            self.advance();
                        }
                    }
                    Some(c) if c == quote => {
                        terminated = true;
                        break;
                    }
                    Some(c) => {
                        push_wtf8_codepoint(&mut w, c as u32);
                    }
                }
            }
            if !terminated {
                return Token::LexerError(format!(
                    "L{}:{}: unterminated string literal (detected at line {})",
                    start_line, start_col, self.line
                ));
            }
        }

        // Emit SyntaxWarning for invalid escape sequences (non-raw only)
        if crate::modules::warning_is_error_mode() {
            if let Some(&esc) = invalid_escapes.first() {
                return Token::LexerError(format!(
                    "\"\\{}\" is an invalid escape sequence. Did you mean \"\\\\{}\"? A raw string is also an option.",
                    esc, esc
                ));
            }
        } else {
            for esc in invalid_escapes {
                self.emit_syntax_warning(
                    &format!("\"\\{}\" is an invalid escape sequence. Did you mean \"\\\\{}\"? A raw string is also an option.", esc, esc),
                    "SyntaxWarning",
                );
            }
        }

        self.emit(Token::String(PyStr::from_wtf8(w)))
    }
}
