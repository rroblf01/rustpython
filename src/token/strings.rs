use super::{Lexer, Token};
use super::unicode_name_to_char;

impl Lexer {
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
                            // See the matching fix in read_string above for
                            // why this needs to consume up to 2 more octal
                            // digits, not just recognize a bare `\0`.
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
                            Some(c) => {
                                bytes.push(b'\\');
                                bytes.push(c as u8);
                            }
                            None => bytes.push(b'\\'),
                        }
                    }
                    Some(c) if c == '\\' && raw => {
                        // See the matching fix/comment in read_string: a
                        // backslash in a raw bytes literal still escapes
                        // exactly the next char for tokenizing purposes, and
                        // that char must be consumed here so a following
                        // quote isn't misread as the terminator.
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
                    "unterminated triple-quoted string literal (detected at line {})",
                    self.line
                ));
            }
        } else {
            loop {
                match self.advance() {
                    None => break,
                    Some(c) if c == '\n' => {
                        return Token::LexerError(format!(
                            "unterminated string literal (detected at line {})",
                            self.line - 1
                        ));
                    }
                    Some(c) if c == '\\' && !raw => {
                        let next = self.advance();
                        match next {
                            Some('n') => bytes.push(b'\n'),
                            Some('t') => bytes.push(b'\t'),
                            Some('r') => bytes.push(b'\r'),
                            Some('\\') => bytes.push(b'\\'),
                            Some('\'') => bytes.push(b'\''),
                            Some('"') => bytes.push(b'"'),
                            // See the matching fix in read_string above for
                            // why this needs to consume up to 2 more octal
                            // digits, not just recognize a bare `\0`.
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

        Token::Bytes(bytes)
    }

    pub(crate) fn read_string(&mut self, quote: char, raw: bool, fstring: bool) -> Token {
        let mut s = String::new();
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
                            Some('n') => s.push('\n'),
                            Some('t') => s.push('\t'),
                            Some('r') => s.push('\r'),
                            Some('\\') => s.push('\\'),
                            Some('\'') => s.push('\''),
                            Some('"') => s.push('"'),
                            // Python string literals accept `\ooo` — 1 to 3
                            // octal digits, the first being the one already
                            // consumed here — as a byte/char escape (e.g.
                            // real code, CPython's own `email._policybase`:
                            // `"[\041-\071\073-\176]+$"`). Previously only a
                            // bare `\0` (no further digits) was handled,
                            // pushing NUL and leaving any following digits
                            // as literal characters — so `\041` produced
                            // `'\x00' + '4' + '1'` (3 chars) instead of the
                            // correct single `'!'` (0o41 = 33).
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
                                s.push((value as u8) as char);
                            }
                            Some('a') => s.push('\x07'),
                            Some('b') => s.push('\x08'),
                            Some('f') => s.push('\x0c'),
                            Some('v') => s.push('\x0b'),
                            Some('x') => {
                                let h1 = self.advance().unwrap_or('0');
                                let h2 = self.advance().unwrap_or('0');
                                let val =
                                    u8::from_str_radix(&format!("{}{}", h1, h2), 16).unwrap_or(0);
                                s.push(val as char);
                            }
                            Some('u') => {
                                let digits: String =
                                    (0..4).map(|_| self.advance().unwrap_or('0')).collect();
                                let val = u32::from_str_radix(&digits, 16).unwrap_or(0xFFFD);
                                s.push(std::char::from_u32(val).unwrap_or('\u{FFFD}'));
                            }
                            Some('U') => {
                                let digits: String =
                                    (0..8).map(|_| self.advance().unwrap_or('0')).collect();
                                let val = u32::from_str_radix(&digits, 16).unwrap_or(0xFFFD);
                                s.push(std::char::from_u32(val).unwrap_or('\u{FFFD}'));
                            }
                            Some('N') => {
                                if self.advance() == Some('{') {
                                    let mut name = String::new();
                                    loop {
                                        match self.advance() {
                                            Some('}') => break,
                                            Some(c) => name.push(c),
                                            None => break,
                                        }
                                    }
                                    let ch = unicode_name_to_char(&name).unwrap_or('\u{FFFD}');
                                    s.push(ch);
                                } else {
                                    s.push('\\');
                                    s.push('N');
                                }
                            }
                            Some(c) if c == '\n' => {}
                            Some(c) => {
                                invalid_escapes.push(c);
                                s.push('\\');
                                s.push(c);
                            }
                            None => s.push('\\'),
                        }
                    }
                    Some(c) if c == '\\' && raw => {
                        // See the matching fix/comment below in the
                        // non-triple branch: a backslash in a raw string
                        // always escapes exactly the next char for
                        // tokenizing purposes, which must be consumed here.
                        s.push(c);
                        if let Some(next) = self.advance() {
                            s.push(next);
                        }
                    }
                    Some(c) if c == quote => {
                        if self.peek() == Some(quote) && self.peek_ahead(1) == Some(quote) {
                            self.advance();
                            self.advance();
                            terminated = true;
                            break;
                        }
                        s.push(c);
                    }
                    Some(c) => {
                        s.push(c);
                    }
                }
            }
            if !terminated {
                return Token::LexerError(format!(
                    "unterminated triple-quoted string literal (detected at line {})",
                    self.line
                ));
            }
        } else {
            loop {
                match self.advance() {
                    None => break,
                    Some(c) if c == '\\' && !raw => {
                        let next = self.advance();
                        match next {
                            Some('n') => s.push('\n'),
                            Some('t') => s.push('\t'),
                            Some('r') => s.push('\r'),
                            Some('\\') => s.push('\\'),
                            Some('\'') => s.push('\''),
                            Some('"') => s.push('"'),
                            // Python string literals accept `\ooo` — 1 to 3
                            // octal digits, the first being the one already
                            // consumed here — as a byte/char escape (e.g.
                            // real code, CPython's own `email._policybase`:
                            // `"[\041-\071\073-\176]+$"`). Previously only a
                            // bare `\0` (no further digits) was handled,
                            // pushing NUL and leaving any following digits
                            // as literal characters — so `\041` produced
                            // `'\x00' + '4' + '1'` (3 chars) instead of the
                            // correct single `'!'` (0o41 = 33).
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
                                s.push((value as u8) as char);
                            }
                            Some('a') => s.push('\x07'),
                            Some('b') => s.push('\x08'),
                            Some('f') => s.push('\x0c'),
                            Some('v') => s.push('\x0b'),
                            Some('x') => {
                                let h1 = self.advance().unwrap_or('0');
                                let h2 = self.advance().unwrap_or('0');
                                let val =
                                    u8::from_str_radix(&format!("{}{}", h1, h2), 16).unwrap_or(0);
                                s.push(val as char);
                            }
                            Some('u') => {
                                let digits: String =
                                    (0..4).map(|_| self.advance().unwrap_or('0')).collect();
                                let val = u32::from_str_radix(&digits, 16).unwrap_or(0xFFFD);
                                s.push(std::char::from_u32(val).unwrap_or('\u{FFFD}'));
                            }
                            Some('U') => {
                                let digits: String =
                                    (0..8).map(|_| self.advance().unwrap_or('0')).collect();
                                let val = u32::from_str_radix(&digits, 16).unwrap_or(0xFFFD);
                                s.push(std::char::from_u32(val).unwrap_or('\u{FFFD}'));
                            }
                            Some('N') => {
                                if self.advance() == Some('{') {
                                    let mut name = String::new();
                                    loop {
                                        match self.advance() {
                                            Some('}') => break,
                                            Some(c) => name.push(c),
                                            None => break,
                                        }
                                    }
                                    let ch = unicode_name_to_char(&name).unwrap_or('\u{FFFD}');
                                    s.push(ch);
                                } else {
                                    s.push('\\');
                                    s.push('N');
                                }
                            }
                            Some(c) if c == '\n' => {}
                            Some(c) => {
                                invalid_escapes.push(c);
                                s.push('\\');
                                s.push(c);
                            }
                            None => s.push('\\'),
                        }
                    }
                    Some(c) if c == '\\' && raw => {
                        // A backslash in a raw string always escapes exactly
                        // the next character for tokenizing purposes (so
                        // e.g. \" doesn't end the string), while keeping
                        // both characters literally in the content. The
                        // escaped character MUST be consumed here — a
                        // peek-only check (only swallowing the following
                        // char when it happens to be the quote) misparses
                        // `\\"` : the first backslash would leave the
                        // second backslash to be examined fresh next
                        // iteration, which would then wrongly swallow the
                        // real closing quote as content instead of
                        // terminating the string.
                        s.push(c);
                        if let Some(next) = self.advance() {
                            s.push(next);
                        }
                    }
                    Some(c) if c == '{' && fstring => {
                        if self.peek() == Some('{') {
                            s.push('{');
                            self.advance();
                        } else {
                            s.push_str("{...}");
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
                            s.push('}');
                            self.advance();
                        }
                    }
                    Some(c) if c == quote => {
                        terminated = true;
                        break;
                    }
                    Some(c) => s.push(c),
                }
            }
            if !terminated {
                return Token::LexerError(format!(
                    "unterminated string literal (detected at line {})",
                    self.line
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
                crate::modules::warnings_emit(
                    &format!("\"\\{}\" is an invalid escape sequence. Did you mean \"\\\\{}\"? A raw string is also an option.", esc, esc),
                    "SyntaxWarning",
                );
            }
        }

        Token::String(s)
    }
}
