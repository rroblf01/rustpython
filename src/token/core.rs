use super::{Lexer, Token};
use super::unicode_name_to_char;
use unicode_normalization::UnicodeNormalization;

impl Lexer {
    pub fn next_token(&mut self) -> Token {
        // Emit any lexer-detected error (e.g. unindent does not match).
        if let Some(err) = self.pending_error.take() {
            return Token::LexerError(err);
        }
        // If we have pending f-string expression tokens, return them
        if let Some(tok) = self.fstring_expr_pending.pop() {
            return tok;
        }
        // Check if we're in the middle of emitting f-string parts
        if let Some(_quote) = self.fstring_quote {
            if self.fstring_part_idx < self.fstring_parts.len() {
                let (ref literal, ref expr_text, ref format_spec, conversion) =
                    self.fstring_parts[self.fstring_part_idx];
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
                    // If there's a conversion, push the FStringConversion token
                    if conversion != 0 {
                        to_push.push(Token::FStringConversion(conversion));
                    }
                    // If there's a format spec, push the FormatSpec token
                    if !format_spec.is_empty() {
                        to_push.push(Token::FormatSpec(format_spec.clone()));
                    }
                }
                // If this is the last part, end the f-string
                let is_last = self.fstring_part_idx >= self.fstring_parts.len()
                    || self.fstring_parts[self.fstring_part_idx..]
                        .iter()
                        .all(|(l, e, fs, c)| {
                            l.is_empty() && e.is_empty() && fs.is_empty() && *c == 0
                        });
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
                    if self.peek().is_none() {
                        return Token::LexerError(
                            "unexpected character after line continuation character".to_string(),
                        );
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
                        "f" | "r" | "b" | "u" | "t" | "fr" | "rf" | "br" | "rb"
                    );
                    if is_string_prefix && (self.peek() == Some('"') || self.peek() == Some('\'')) {
                        let quote = self.advance().unwrap();
                        let raw = lower_name.contains('r');
                        let is_fstring = lower_name.contains('f');
                        let is_bytes = lower_name.contains('b');
                        if is_fstring {
                            return self.tokenize_fstring(quote, raw);
                        } else if is_bytes {
                            return self.read_bytes(quote, raw);
                        } else {
                            return self.read_string(quote, raw, false);
                        }
                    }
                    // PEP 3131: identifiers are NFKC-normalized. This handles
                    // compatibility characters (e.g. MICRO SIGN U+00B5 -> GREEK MU
                    // U+03BC, and mathematical fraktur -> ASCII) so that
                    // `µ` and `μ` are the same identifier, and
                    // `𝔘𝔫𝔦𝔠𝔬𝔡𝔢` normalizes to `Unicode`.
                    let normalized: String = name.nfkc().collect();
                    return match normalized.as_str() {
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
                }

                // Operators and delimiters
                '+' => {
                    if self.advance_if('=') {
                        return Token::PlusEqual;
                    } else {
                        return Token::Plus;
                    }
                }
                '-' => {
                    if self.advance_if('=') {
                        return Token::MinusEqual;
                    } else if self.advance_if('>') {
                        return Token::Arrow;
                    } else {
                        return Token::Minus;
                    }
                }
                '*' => {
                    if self.advance_if('*') {
                        if self.advance_if('=') {
                            return Token::DoubleStarEqual;
                        } else {
                            return Token::DoubleStar;
                        }
                    } else if self.advance_if('=') {
                        return Token::StarEqual;
                    } else {
                        return Token::Star;
                    }
                }
                '/' => {
                    if self.advance_if('/') {
                        if self.advance_if('=') {
                            return Token::DoubleSlashEqual;
                        } else {
                            return Token::DoubleSlash;
                        }
                    } else if self.advance_if('=') {
                        return Token::SlashEqual;
                    } else {
                        return Token::Slash;
                    }
                }
                '%' => {
                    if self.advance_if('=') {
                        return Token::PercentEqual;
                    } else {
                        return Token::Percent;
                    }
                }
                '@' => {
                    if self.advance_if('=') {
                        return Token::AtEqual;
                    } else {
                        return Token::At;
                    }
                }
                '&' => {
                    if self.advance_if('=') {
                        return Token::AmpersandEqual;
                    } else {
                        return Token::Ampersand;
                    }
                }
                '|' => {
                    if self.advance_if('=') {
                        return Token::PipeEqual;
                    } else {
                        return Token::Pipe;
                    }
                }
                '^' => {
                    if self.advance_if('=') {
                        return Token::CaretEqual;
                    } else {
                        return Token::Caret;
                    }
                }
                '~' => {
                    return Token::Tilde;
                }
                '<' => {
                    if self.advance_if('<') {
                        if self.advance_if('=') {
                            return Token::LeftShiftEqual;
                        } else {
                            return Token::LeftShift;
                        }
                    } else if self.advance_if('=') {
                        return Token::LessEqual;
                    } else if self.advance_if('>') {
                        // `<>` — legacy not-equal. Only valid with the
                        // barry_as_FLUFL future flag; the PARSER decides
                        // (accepts it then, rejects it otherwise).
                        return Token::LessGreater;
                    } else {
                        return Token::Less;
                    }
                }
                '>' => {
                    if self.advance_if('>') {
                        if self.advance_if('=') {
                            return Token::RightShiftEqual;
                        } else {
                            return Token::RightShift;
                        }
                    } else if self.advance_if('=') {
                        return Token::GreaterEqual;
                    } else {
                        return Token::Greater;
                    }
                }
                '=' => {
                    if self.advance_if('=') {
                        return Token::EqualEqual;
                    } else {
                        return Token::Equal;
                    }
                }
                '!' => {
                    if self.advance_if('=') {
                        return Token::NotEqual;
                    } else {
                        return Token::Name("!".to_string());
                    }
                }
                '(' => {
                    self.paren_level += 1;
                    return Token::LeftParen;
                }
                ')' => {
                    if self.paren_level > 0 {
                        self.paren_level -= 1;
                    }
                    return Token::RightParen;
                }
                '[' => {
                    self.paren_level += 1;
                    return Token::LeftBracket;
                }
                ']' => {
                    if self.paren_level > 0 {
                        self.paren_level -= 1;
                    }
                    return Token::RightBracket;
                }
                '{' => {
                    self.paren_level += 1;
                    return Token::LeftBrace;
                }
                '}' => {
                    if self.paren_level > 0 {
                        self.paren_level -= 1;
                    }
                    return Token::RightBrace;
                }
                ',' => {
                    return Token::Comma;
                }
                ':' => {
                    if self.advance_if('=') {
                        return Token::Walrus;
                    } else {
                        return Token::Colon;
                    }
                }
                ';' => {
                    return Token::Semicolon;
                }
                '.' => {
                    if self.peek() == Some('.') && self.peek_ahead(1) == Some('.') {
                        self.advance();
                        self.advance();
                        return Token::Ellipsis;
                    } else if self.peek().map_or(false, |c| c.is_ascii_digit()) {
                        // A float literal with no leading digit (`.995`,
                        // `.5e10`) — real, standard Python syntax (real code:
                        // CPython's own test suite uses `.995 * digits`),
                        // previously not recognized at all since this
                        // dispatch only ever produced a bare Dot/Ellipsis
                        // token regardless of what followed.
                        return self.read_number('.');
                    } else {
                        return Token::Dot;
                    }
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
                    if self.peek() == Some('\n') {
                        self.advance();
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
                self.pending_error = Some(
                    "unindent does not match any outer indentation level".to_string(),
                );
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

    fn tokenize_fstring(&mut self, quote: char, raw: bool) -> Token {
        // Read the entire f-string, splitting into literal and expression parts
        // Each part: (literal_text, expr_text, format_spec_text, conversion)
        let triple = self.peek() == Some(quote) && self.peek_ahead(1) == Some(quote);
        if triple {
            self.advance();
            self.advance();
        }
        let mut parts: Vec<(String, String, String, u8)> = Vec::new();
        let mut literal = String::new();
        let mut terminated = false;
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
                        // forms the doubled-brace escape.
                        literal.push('\\');
                        continue;
                    }
                    let next = self.advance();
                    match next {
                        Some('N') if self.peek() == Some('{') => {
                            // \N{...} unicode name escape: the '{' must NOT
                            // be treated as an f-string field start.
                            self.advance();
                            let mut name = String::new();
                            loop {
                                match self.advance() {
                                    Some('}') => break,
                                    Some(c) => name.push(c),
                                    None => break,
                                }
                            }
                            let ch = unicode_name_to_char(&name).unwrap_or('\u{FFFD}');
                            literal.push(ch);
                        }
                        Some(c) => {
                            literal.push(match c {
                                'n' => '\n',
                                't' => '\t',
                                'r' => '\r',
                                '\\' => '\\',
                                '\'' => '\'',
                                '"' => '"',
                                '{' => '{',
                                '}' => '}',
                                c => c,
                            });
                        }
                        None => literal.push('\\'),
                    }
                }
                Some(c) if c == '{' => {
                    if self.peek() == Some('{') {
                        self.advance();
                        literal.push('{');
                    } else {
                        // Start of expression — read until matching '}'
                        let mut depth = 1;
                        let mut expr = String::new();
                        let mut conversion: u8 = 0;
                        let mut format_spec = String::new();
                        let mut debug: bool = false;
                        let mut state: u8 = 0; // 0=expr, 1=after_conv_marker, 2=format_spec
                        let mut bracket_depth: u32 = 0; // track ()[] nesting
                        let mut str_char: char = '\0'; // track strings: '\'' or '"' or '\0'
                        let mut triple_str: bool = false; // inside a """ / ''' string
                        while depth > 0 {
                            match self.advance() {
                                Some(c) if str_char != '\0' && c == str_char => {
                                    expr.push(c);
                                    if triple_str {
                                        // A triple-quoted string needs all
                                        // three quotes to close.
                                        if self.peek() == Some(c) && self.peek_ahead(1) == Some(c) {
                                            expr.push(c);
                                            expr.push(c);
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
                                    expr.push(c);
                                    if c == '\\' {
                                        if let Some(next) = self.advance() {
                                            expr.push(next);
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
                                            None | Some('\n') => break,
                                            Some(nc) => {
                                                expr.push(nc);
                                                self.advance();
                                            }
                                        }
                                    }
                                }
                                Some(c @ ('\'' | '"')) if str_char == '\0' && state == 0 => {
                                    // Detect a triple-quoted string (""" / '''):
                                    // the three quotes must all be consumed
                                    // together so the single-quote closer logic
                                    // above doesn't fire on the first one.
                                    if self.peek() == Some(c) && self.peek_ahead(1) == Some(c) {
                                        str_char = c;
                                        triple_str = true;
                                        expr.push(c);
                                        expr.push(c);
                                        expr.push(c);
                                        self.advance();
                                        self.advance();
                                    } else {
                                        str_char = c;
                                        expr.push(c);
                                    }
                                }
                                Some('{') => {
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
                                    depth += 1;
                                    if depth > 1 || state > 0 {
                                        if state == 2 {
                                            format_spec.push('{');
                                        } else {
                                            expr.push('{');
                                        }
                                    }
                                }
                                Some('}') => {
                                    if state == 2 && depth > 1 {
                                        format_spec.push('}');
                                        depth -= 1;
                                    } else {
                                        depth -= 1;
                                        if depth > 0 {
                                            if state == 2 {
                                                format_spec.push('}');
                                            } else {
                                                expr.push('}');
                                            }
                                        }
                                    }
                                }
                                Some('=') if depth == 1 && state == 0 => match self.peek() {
                                    Some('=') => {
                                        expr.push('=');
                                        expr.push('=');
                                        self.advance();
                                    }
                                    Some('}') | Some('!') | Some(':') => {
                                        debug = true;
                                    }
                                    _ => {
                                        expr.push('=');
                                    }
                                },
                                Some('!') if depth == 1 && state == 0 => {
                                    if self.peek() == Some('=') {
                                        expr.push('!');
                                        expr.push('=');
                                        self.advance();
                                    } else {
                                        state = 1;
                                    }
                                }
                                Some('r') if state == 1 => {
                                    conversion = 1;
                                    state = 0;
                                }
                                Some('s') if state == 1 => {
                                    conversion = 2;
                                    state = 0;
                                }
                                Some('a') if state == 1 => {
                                    conversion = 3;
                                    state = 0;
                                }
                                Some(':') if depth == 1 && state == 0 && bracket_depth == 0 => {
                                    if self.peek() == Some('=') {
                                        expr.push(':');
                                        expr.push('=');
                                        self.advance();
                                    } else {
                                        state = 2;
                                    }
                                }
                                Some(c) => {
                                    if state == 1 {
                                        // '!' was not followed by r/s/a — treat as part of expr
                                        expr.push('!');
                                        expr.push(c);
                                        state = 0;
                                    } else if state == 2 {
                                        format_spec.push(c);
                                    } else {
                                        expr.push(c);
                                        // Track bracket nesting for format spec : detection
                                        if c == '(' || c == '[' {
                                            bracket_depth += 1;
                                        } else if c == ')' || c == ']' {
                                            if bracket_depth > 0 {
                                                bracket_depth -= 1;
                                            }
                                        }
                                    }
                                }
                                None => break,
                            }
                        }
                        // If debug mode, prepend "{expr}=" to the literal text
                        if debug {
                            let prefix = format!("{}=", expr);
                            literal.push_str(&prefix);
                            if conversion == 0 {
                                conversion = 1; // Default to !r (repr) for debug
                            }
                        }
                        parts.push((std::mem::take(&mut literal), expr, format_spec, conversion));
                    }
                }
                Some(c) if c == '}' => {
                    if self.peek() == Some('}') {
                        self.advance();
                        literal.push('}');
                    } else {
                        // Stray } outside expression — push as literal
                        literal.push('}');
                    }
                }
                Some(c) if c == quote => {
                    if triple {
                        if self.peek() == Some(quote) && self.peek_ahead(1) == Some(quote) {
                            self.advance();
                            self.advance();
                            terminated = true;
                            break;
                        }
                        literal.push(c);
                    } else {
                        terminated = true;
                        break;
                    }
                }
                Some(c) => literal.push(c),
            }
        }
        if !terminated {
            if triple {
                return Token::LexerError(format!(
                    "unterminated triple-quoted string literal (detected at line {})",
                    self.line
                ));
            } else {
                return Token::LexerError(format!(
                    "unterminated string literal (detected at line {})",
                    self.line
                ));
            }
        }
        parts.push((literal, String::new(), String::new(), 0));

        self.fstring_quote = Some(quote);
        self.fstring_parts = parts;
        self.fstring_part_idx = 0;
        self.fstring_expr_pending = Vec::new();

        Token::FStringStart
    }

    fn tokenize_fstring_expr(&self, text: &str) -> Vec<Token> {
        // Strip leading/trailing whitespace — expressions inside f-strings
        // are not subject to indentation processing
        let text = text.trim();
        // Tokenize an f-string expression text
        let mut lex = Lexer::new(text);
        lex.at_line_start = false;
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
