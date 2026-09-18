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
                                    push_wtf8_codepoint(&mut w, ch as u32);
                                } else {
                                    w.push(b'\\');
                                    w.push(b'N');
                                }
                            }
                            Some(c) if c == '\n' => {}
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
                                    push_wtf8_codepoint(&mut w, ch as u32);
                                } else {
                                    w.push(b'\\');
                                    w.push(b'N');
                                }
                            }
                            Some(c) if c == '\n' => {}
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

        Token::String(PyStr::from_wtf8(w))
    }
}
