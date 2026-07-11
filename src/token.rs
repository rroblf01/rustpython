use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Name(String),
    Number(String),
    String(String),
    Bytes(Vec<u8>),
    FStringStart,
    FStringMiddle(String),
    FStringEnd,
    Indent,
    Dedent,
    Newline,
    EndOfFile,

    // Operators
    Plus,
    Minus,
    Star,
    Slash,
    DoubleStar,
    DoubleSlash,
    Percent,
    At,
    Tilde,
    Pipe,
    Ampersand,
    Caret,
    LeftShift,
    RightShift,
    Less,
    Greater,
    LessEqual,
    GreaterEqual,
    EqualEqual,
    NotEqual,
    Equal,
    PlusEqual,
    MinusEqual,
    StarEqual,
    SlashEqual,
    DoubleStarEqual,
    DoubleSlashEqual,
    PercentEqual,
    PipeEqual,
    AmpersandEqual,
    CaretEqual,
    LeftShiftEqual,
    RightShiftEqual,
    AtEqual,

    // Delimiters
    LeftParen,
    RightParen,
    LeftBracket,
    RightBracket,
    LeftBrace,
    RightBrace,
    Comma,
    Colon,
    Semicolon,
    Dot,
    Arrow,
    Ellipsis,
    Walrus,

    // Keywords
    False,
    None,
    True,
    And,
    As,
    Assert,
    Async,
    Await,
    Break,
    Class,
    Continue,
    Def,
    Del,
    Elif,
    Else,
    Except,
    Finally,
    For,
    From,
    Global,
    If,
    Import,
    In,
    Is,
    Lambda,
    Nonlocal,
    Not,
    Or,
    Pass,
    Raise,
    Return,
    Try,
    While,
    With,
    Yield,

    // Soft keywords
    Match,
    Case,
    TypeKw,
    Underscore,
}

impl fmt::Display for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Token::Name(s) => write!(f, "NAME({})", s),
            Token::Number(s) => write!(f, "NUMBER({})", s),
            Token::String(s) => write!(f, "STRING({:?})", s),
            Token::Bytes(b) => write!(f, "BYTES({:?})", String::from_utf8_lossy(b)),
            t => write!(f, "{:?}", t),
        }
    }
}

pub struct Lexer {
    chars: Vec<char>,
    pos: usize,
    line: usize,
    col: usize,
    indent_stack: Vec<usize>,
    pending: Vec<Token>,
    at_line_start: bool,
    paren_level: usize,
    source: String,
    fstring_quote: Option<char>,
    fstring_parts: Vec<(String, String)>, // (literal, expr_text)
    fstring_part_idx: usize,
    fstring_expr_pending: Vec<Token>,
}

impl Lexer {
    pub fn new(source: &str) -> Self {
        let chars: Vec<char> = source.chars().collect();
        Lexer {
            chars,
            pos: 0,
            line: 1,
            col: 1,
            indent_stack: vec![0],
            pending: Vec::new(),
            at_line_start: true,
            paren_level: 0,
            source: source.to_string(),
            fstring_quote: None,
            fstring_parts: Vec::new(),
            fstring_part_idx: 0,
            fstring_expr_pending: Vec::new(),
        }
    }

    pub fn source_text(&self) -> &str {
        &self.source
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn peek_ahead(&self, n: usize) -> Option<char> {
        self.chars.get(self.pos + n).copied()
    }

    fn advance(&mut self) -> Option<char> {
        let ch = self.chars.get(self.pos).copied()?;
        self.pos += 1;
        if ch == '\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        Some(ch)
    }

    fn advance_if(&mut self, expected: char) -> bool {
        if self.peek() == Some(expected) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn skip_whitespace(&mut self) {
        while self.peek() == Some(' ') || self.peek() == Some('\t') {
            self.advance();
        }
    }

    fn is_hex_char(c: char) -> bool {
        c.is_ascii_hexdigit()
    }

    fn is_oct_char(c: char) -> bool {
        matches!(c, '0'..='7')
    }

    fn is_bin_char(c: char) -> bool {
        matches!(c, '0' | '1')
    }

    fn is_identifier_start(c: char) -> bool {
        c.is_ascii_alphabetic() || c == '_'
    }

    fn is_identifier_continue(c: char) -> bool {
        c.is_ascii_alphanumeric() || c == '_'
    }

    fn read_number(&mut self, first: char) -> Token {
        let mut s = String::new();
        s.push(first);

        if first == '0' {
            let next = self.peek();
            match next {
                Some('x' | 'X') => {
                    s.push(self.advance().unwrap());
                    while self.peek().map_or(false, |c| Self::is_hex_char(c) || c == '_') {
                        s.push(self.advance().unwrap());
                    }
                    return Token::Number(s);
                }
                Some('o' | 'O') => {
                    s.push(self.advance().unwrap());
                    while self.peek().map_or(false, |c| Self::is_oct_char(c) || c == '_') {
                        s.push(self.advance().unwrap());
                    }
                    return Token::Number(s);
                }
                Some('b' | 'B') => {
                    s.push(self.advance().unwrap());
                    while self.peek().map_or(false, |c| Self::is_bin_char(c) || c == '_') {
                        s.push(self.advance().unwrap());
                    }
                    return Token::Number(s);
                }
                _ => {}
            }
        }

        let mut is_float = false;
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() || c == '_' {
                s.push(self.advance().unwrap());
            } else if c == '.' && !is_float {
                let next = self.peek_ahead(1);
                if next == Some('.') {
                    break;
                }
                is_float = true;
                s.push(self.advance().unwrap());
            } else if c == 'e' || c == 'E' {
                s.push(self.advance().unwrap());
                if self.peek() == Some('+') || self.peek() == Some('-') {
                    s.push(self.advance().unwrap());
                }
                is_float = true;
            } else if c == 'j' || c == 'J' {
                s.push(self.advance().unwrap());
                break;
            } else {
                break;
            }
        }
        Token::Number(s)
    }

    fn read_bytes(&mut self, quote: char) -> Token {
        let mut bytes = Vec::new();
        let triple = self.peek() == Some(quote)
            && self.peek_ahead(1) == Some(quote);

        if triple {
            self.advance();
            self.advance();
            loop {
                match self.advance() {
                    None => break,
                    Some(c) if c == '\\' => {
                        let next = self.advance();
                        match next {
                            Some('n') => bytes.push(b'\n'),
                            Some('t') => bytes.push(b'\t'),
                            Some('r') => bytes.push(b'\r'),
                            Some('\\') => bytes.push(b'\\'),
                            Some('\'') => bytes.push(b'\''),
                            Some('"') => bytes.push(b'"'),
                            Some('x') => {
                                let h1 = self.advance().unwrap_or('0');
                                let h2 = self.advance().unwrap_or('0');
                                let val = u8::from_str_radix(&format!("{}{}", h1, h2), 16).unwrap_or(0);
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
                    Some(c) if c == quote => {
                        if self.peek() == Some(quote) && self.peek_ahead(1) == Some(quote) {
                            self.advance();
                            self.advance();
                            break;
                        }
                        bytes.push(c as u8);
                    }
                    Some(c) => {
                        bytes.push(c as u8);
                    }
                }
            }
        } else {
            loop {
                match self.advance() {
                    None => break,
                    Some(c) if c == '\\' => {
                        let next = self.advance();
                        match next {
                            Some('n') => bytes.push(b'\n'),
                            Some('t') => bytes.push(b'\t'),
                            Some('r') => bytes.push(b'\r'),
                            Some('\\') => bytes.push(b'\\'),
                            Some('\'') => bytes.push(b'\''),
                            Some('"') => bytes.push(b'"'),
                            Some('x') => {
                                let h1 = self.advance().unwrap_or('0');
                                let h2 = self.advance().unwrap_or('0');
                                let val = u8::from_str_radix(&format!("{}{}", h1, h2), 16).unwrap_or(0);
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
                    Some(c) if c == quote => break,
                    Some(c) => {
                        bytes.push(c as u8);
                    }
                }
            }
        }

        Token::Bytes(bytes)
    }

    fn read_string(&mut self, quote: char, raw: bool, fstring: bool) -> Token {
        let mut s = String::new();
        let triple = self.peek() == Some(quote)
            && self.peek_ahead(1) == Some(quote);

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
                            Some('0') => s.push('\0'),
                            Some('a') => s.push('\x07'),
                            Some('b') => s.push('\x08'),
                            Some('f') => s.push('\x0c'),
                            Some('v') => s.push('\x0b'),
                            Some('x') => {
                                let h1 = self.advance().unwrap_or('0');
                                let h2 = self.advance().unwrap_or('0');
                                let val = u8::from_str_radix(&format!("{}{}", h1, h2), 16).unwrap_or(0);
                                s.push(val as char);
                            }
                            Some('u') => {
                                let digits: String = (0..4).map(|_| self.advance().unwrap_or('0')).collect();
                                let val = u32::from_str_radix(&digits, 16).unwrap_or(0xFFFD);
                                s.push(std::char::from_u32(val).unwrap_or('\u{FFFD}'));
                            }
                            Some('U') => {
                                let digits: String = (0..8).map(|_| self.advance().unwrap_or('0')).collect();
                                let val = u32::from_str_radix(&digits, 16).unwrap_or(0xFFFD);
                                s.push(std::char::from_u32(val).unwrap_or('\u{FFFD}'));
                            }
                            Some(c) if c == '\n' => {}
                            Some(c) => {
                                s.push('\\');
                                s.push(c);
                            }
                            None => s.push('\\'),
                        }
                    }
                    Some(c) if c == quote => {
                        if self.peek() == Some(quote) && self.peek_ahead(1) == Some(quote) {
                            self.advance();
                            self.advance();
                            break;
                        }
                        s.push(c);
                    }
                    Some(c) => {
                        s.push(c);
                    }
                }
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
                            Some('0') => s.push('\0'),
                            Some('a') => s.push('\x07'),
                            Some('b') => s.push('\x08'),
                            Some('f') => s.push('\x0c'),
                            Some('v') => s.push('\x0b'),
                            Some('x') => {
                                let h1 = self.advance().unwrap_or('0');
                                let h2 = self.advance().unwrap_or('0');
                                let val = u8::from_str_radix(&format!("{}{}", h1, h2), 16).unwrap_or(0);
                                s.push(val as char);
                            }
                            Some('u') => {
                                let digits: String = (0..4).map(|_| self.advance().unwrap_or('0')).collect();
                                let val = u32::from_str_radix(&digits, 16).unwrap_or(0xFFFD);
                                s.push(std::char::from_u32(val).unwrap_or('\u{FFFD}'));
                            }
                            Some('U') => {
                                let digits: String = (0..8).map(|_| self.advance().unwrap_or('0')).collect();
                                let val = u32::from_str_radix(&digits, 16).unwrap_or(0xFFFD);
                                s.push(std::char::from_u32(val).unwrap_or('\u{FFFD}'));
                            }
                            Some(c) if c == '\n' => {}
                            Some(c) => {
                                s.push('\\');
                                s.push(c);
                            }
                            None => s.push('\\'),
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
                    Some(c) if c == quote => break,
                    Some(c) => s.push(c),
                }
            }
        }

        Token::String(s)
    }

    pub fn next_token(&mut self) -> Token {
        // If we have pending f-string expression tokens, return them
        if let Some(tok) = self.fstring_expr_pending.pop() {
            return tok;
        }
        // Check if we're in the middle of emitting f-string parts
        if let Some(_quote) = self.fstring_quote {
            if self.fstring_part_idx < self.fstring_parts.len() {
                let (ref literal, ref expr_text) = self.fstring_parts[self.fstring_part_idx];
                self.fstring_part_idx += 1;
                // to_push holds tokens in OUTPUT order (first to last)
                let mut to_push: Vec<Token> = Vec::new();
                // Push literal text (if any) — this should come FIRST
                if !literal.is_empty() {
                    to_push.push(Token::FStringMiddle(literal.clone()));
                }
                // Push expression tokens (if any) — these come AFTER the literal
                if !expr_text.is_empty() {
                    let expr_tokens = self.tokenize_fstring_expr(expr_text);
                    to_push.extend(expr_tokens);
                }
                // If this is the last part, end the f-string
                let is_last = self.fstring_part_idx >= self.fstring_parts.len()
                    || self.fstring_parts[self.fstring_part_idx..].iter().all(|(l, e)| l.is_empty() && e.is_empty());
                if is_last {
                    to_push.push(Token::FStringEnd);
                    self.fstring_quote = None;
                }
                // Push to pending stack in REVERSE (so they come out in correct order)
                for t in to_push.into_iter().rev() {
                    self.fstring_expr_pending.push(t);
                }
                return self.next_token();
            }
            // Cleanup if we somehow reach here without emitting FStringEnd
            self.fstring_quote = None;
            self.fstring_parts = Vec::new();
            self.fstring_part_idx = 0;
            return self.next_token(); // Try again with fstring_quote cleared
        }

        if let Some(tok) = self.pending.pop() {
            return tok;
        }

        if self.at_line_start && self.paren_level == 0 {
            self.handle_indent();
            self.at_line_start = false;
            if let Some(tok) = self.pending.pop() {
                return tok;
            }
        }

        loop {
            let ch = match self.advance() {
                Some(c) => c,
                None => return Token::EndOfFile,
            };

            match ch {
                ' ' | '\t' => {
                    continue;
                }
                '#' => {
                    while self.peek() != Some('\n') && self.peek().is_some() {
                        self.advance();
                    }
                    if self.peek().is_some() {
                        self.at_line_start = true;
                    }
                    return Token::Newline;
                }
                '\n' => {
                    if self.paren_level > 0 {
                        continue;
                    }
                    self.at_line_start = true;
                    return Token::Newline;
                }
                '\\' => {
                    if self.peek() == Some('\n') {
                        self.advance();
                        continue;
                    }
                    return Token::Name("\\".to_string());
                }
                '\r' => continue,

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
                    // Check for f-prefixed strings (f"..." or f'...')
                    if (name == "f" || name == "F") && (self.peek() == Some('"') || self.peek() == Some('\'')) {
                        let quote = self.advance().unwrap();
                        return self.tokenize_fstring(quote);
                    }
                    // Check for bytes literals (b"..." or b'...')
                    if (name == "b" || name == "B") && (self.peek() == Some('"') || self.peek() == Some('\'')) {
                        let quote = self.advance().unwrap();
                        return self.read_bytes(quote);
                    }
                    return match name.as_str() {
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
                        "match" => Token::Match,
                        "case" => Token::Case,
                        "_" => Token::Underscore,
                        _ => Token::Name(name),
                    };
                }

                // Operators and delimiters
                '+' => {
                    if self.advance_if('=') { return Token::PlusEqual }
                    else { return Token::Plus }
                }
                '-' => {
                    if self.advance_if('=') { return Token::MinusEqual }
                    else if self.advance_if('>') { return Token::Arrow }
                    else { return Token::Minus }
                }
                '*' => {
                    if self.advance_if('*') {
                        if self.advance_if('=') { return Token::DoubleStarEqual }
                        else { return Token::DoubleStar }
                    } else if self.advance_if('=') { return Token::StarEqual }
                    else { return Token::Star }
                }
                '/' => {
                    if self.advance_if('/') {
                        if self.advance_if('=') { return Token::DoubleSlashEqual }
                        else { return Token::DoubleSlash }
                    } else if self.advance_if('=') { return Token::SlashEqual }
                    else { return Token::Slash }
                }
                '%' => {
                    if self.advance_if('=') { return Token::PercentEqual }
                    else { return Token::Percent }
                }
                '@' => {
                    if self.advance_if('=') { return Token::AtEqual }
                    else { return Token::At }
                }
                '&' => {
                    if self.advance_if('=') { return Token::AmpersandEqual }
                    else { return Token::Ampersand }
                }
                '|' => {
                    if self.advance_if('=') { return Token::PipeEqual }
                    else { return Token::Pipe }
                }
                '^' => {
                    if self.advance_if('=') { return Token::CaretEqual }
                    else { return Token::Caret }
                }
                '~' => { return Token::Tilde; },
                '<' => {
                    if self.advance_if('<') {
                        if self.advance_if('=') { return Token::LeftShiftEqual }
                        else { return Token::LeftShift }
                    } else if self.advance_if('=') { return Token::LessEqual }
                    else { return Token::Less }
                }
                '>' => {
                    if self.advance_if('>') {
                        if self.advance_if('=') { return Token::RightShiftEqual }
                        else { return Token::RightShift }
                    } else if self.advance_if('=') { return Token::GreaterEqual }
                    else { return Token::Greater }
                }
                '=' => {
                    if self.advance_if('=') { return Token::EqualEqual }
                    else { return Token::Equal }
                }
                '!' => {
                    if self.advance_if('=') { return Token::NotEqual }
                    else { return Token::Name("!".to_string()) }
                }
                '(' => { self.paren_level += 1; return Token::LeftParen; }
                ')' => { self.paren_level -= 1; return Token::RightParen; }
                '[' => { self.paren_level += 1; return Token::LeftBracket; }
                ']' => { self.paren_level -= 1; return Token::RightBracket; }
                '{' => { self.paren_level += 1; return Token::LeftBrace; }
                '}' => { self.paren_level -= 1; return Token::RightBrace; }
                ',' => { return Token::Comma; },
                ':' => {
                    if self.advance_if('=') { return Token::Walrus }
                    else { return Token::Colon }
                }
                ';' => { return Token::Semicolon; },
                '.' => {
                    if self.peek() == Some('.') && self.peek_ahead(1) == Some('.') {
                        self.advance(); self.advance();
                        return Token::Ellipsis;
                    } else {
                        return Token::Dot;
                    }
                }

                _ => return Token::Name(ch.to_string()),
            }
        }
    }

    fn handle_indent(&mut self) {
        let mut indent = 0;
        loop {
            match self.peek() {
                Some(' ') => { indent += 1; self.advance(); }
                Some('\t') => { indent += 8; self.advance(); }
                Some('#') => {
                    while self.peek() != Some('\n') && self.peek().is_some() {
                        self.advance();
                    }
                    if self.peek().is_some() {
                        self.advance();
                        self.at_line_start = true;
                    }
                    break;
                }
                Some('\n') => {
                    self.advance();
                    indent = 0;
                    self.at_line_start = true;
                    continue;
                }
                Some('\r') => { self.advance(); continue; }
                Some('\\') => {
                    self.advance();
                    if self.peek() == Some('\n') { self.advance(); }
                    indent = 0;
                    continue;
                }
                _ => break,
            }
        }
        if self.peek().is_none() || self.peek() == Some('\n') {
            return;
        }
        let current = *self.indent_stack.last().unwrap();
        if indent > current {
            self.indent_stack.push(indent);
            self.pending.push(Token::Indent);
        } else if indent < current {
            while let Some(&level) = self.indent_stack.last() {
                if level == indent {
                    break;
                }
                self.indent_stack.pop();
                self.pending.push(Token::Dedent);
            }
        }
    }

    fn tokenize_fstring(&mut self, quote: char) -> Token {
        // Read the entire f-string, splitting into literal and expression parts
        let mut parts: Vec<(String, String)> = Vec::new();
        let mut literal = String::new();
        loop {
            match self.advance() {
                None => break,
                Some(c) if c == '\\' => {
                    let next = self.advance();
                    literal.push(match next {
                        Some('n') => '\n',
                        Some('t') => '\t',
                        Some('r') => '\r',
                        Some('\\') => '\\',
                        Some('\'') => '\'',
                        Some('"') => '"',
                        Some('{') => '{',
                        Some('}') => '}',
                        Some(c) => c,
                        None => '\\',
                    });
                }
                Some(c) if c == '{' => {
                    if self.peek() == Some('{') {
                        self.advance();
                        literal.push('{');
                    } else {
                        // Start of expression
                        let mut depth = 1;
                        let mut expr = String::new();
                        while depth > 0 {
                            match self.advance() {
                                Some('{') => { depth += 1; if depth > 1 { expr.push('{'); } }
                                Some('}') => { depth -= 1; if depth > 0 { expr.push('}'); } }
                                Some(c) => expr.push(c),
                                None => break,
                            }
                        }
                        parts.push((std::mem::take(&mut literal), expr));
                    }
                }
                Some(c) if c == '}' => {
                    if self.peek() == Some('}') {
                        self.advance();
                        literal.push('}');
                    }
                }
                Some(c) if c == quote => break,
                Some(c) => literal.push(c),
            }
        }
        parts.push((literal, String::new()));

        self.fstring_quote = Some(quote);
        self.fstring_parts = parts;
        self.fstring_part_idx = 0;
        self.fstring_expr_pending = Vec::new();

        Token::FStringStart
    }

    fn tokenize_fstring_expr(&self, text: &str) -> Vec<Token> {
        // Tokenize an f-string expression text
        let mut lex = Lexer::new(text);
        let mut tokens = Vec::new();
        loop {
            let tok = lex.next_token();
            if tok == Token::EndOfFile {
                break;
            }
            if tok == Token::Newline {
                continue;
            }
            tokens.push(tok);
        }
        tokens
    }

    pub fn get_line_col(&self) -> (usize, usize) {
        (self.line, self.col)
    }
}
