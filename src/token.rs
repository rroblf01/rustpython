use std::fmt;

fn unicode_name_to_char(name: &str) -> Option<char> {
    Some(match name {
        "EM SPACE" => '\u{2003}',
        "EN SPACE" => '\u{2002}',
        "NOT SIGN" => '\u{00AC}',
        "EURO SIGN" => '\u{20AC}',
        "POUND SIGN" => '\u{00A3}',
        "COPYRIGHT SIGN" => '\u{00A9}',
        "REGISTERED SIGN" => '\u{00AE}',
        "SECTION SIGN" => '\u{00A7}',
        "BULLET" => '\u{2022}',
        "HORIZONTAL ELLIPSIS" => '\u{2026}',
        "LEFTWARDS ARROW" => '\u{2190}',
        "UPWARDS ARROW" => '\u{2191}',
        "RIGHTWARDS ARROW" => '\u{2192}',
        "DOWNWARDS ARROW" => '\u{2193}',
        "LEFT DOUBLE QUOTATION MARK" => '\u{201C}',
        "RIGHT DOUBLE QUOTATION MARK" => '\u{201D}',
        "LEFT SINGLE QUOTATION MARK" => '\u{2018}',
        "RIGHT SINGLE QUOTATION MARK" => '\u{2019}',
        "LATIN SMALL LETTER SHARP S" => '\u{00DF}',
        "AMPERSAND" => '\u{0026}',
        "GREEK CAPITAL LETTER DELTA" => '\u{0394}',
        "LEFT CURLY BRACKET" => '\u{007B}',
        "RIGHT CURLY BRACKET" => '\u{007D}',
        "MICRO SIGN" => '\u{00B5}',
        "DEGREE SIGN" => '\u{00B0}',
        "PLUS-MINUS SIGN" => '\u{00B1}',
        "SUPERSCRIPT TWO" => '\u{00B2}',
        "SUPERSCRIPT THREE" => '\u{00B3}',
        "ACUTE ACCENT" => '\u{00B4}',
        "MICRO SIGN" => '\u{00B5}',
        "PILCROW SIGN" => '\u{00B6}',
        "MIDDLE DOT" => '\u{00B7}',
        "CEDILLA" => '\u{00B8}',
        "SUPERSCRIPT ONE" => '\u{00B9}',
        "MASCULINE ORDINAL INDICATOR" => '\u{00BA}',
        "RIGHT-POINTING DOUBLE ANGLE QUOTATION MARK" => '\u{00BB}',
        "VULGAR FRACTION ONE QUARTER" => '\u{00BC}',
        "VULGAR FRACTION ONE HALF" => '\u{00BD}',
        "VULGAR FRACTION THREE QUARTERS" => '\u{00BE}',
        "INVERTED QUESTION MARK" => '\u{00BF}',
        _ => return None,
    })
}

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Name(String),
    Number(String),
    String(String),
    Bytes(Vec<u8>),
    FStringStart,
    FStringMiddle(String),
    FStringEnd,
    FormatSpec(String),
    FStringConversion(u8),
    Indent,
    Dedent,
    Newline,
    EndOfFile,
    /// Lexer-detected error (e.g. unindent does not match) — carries the message.
    LexerError(String),

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
    LessGreater,
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

#[derive(Clone)]
pub struct Lexer {
    chars: Vec<char>,
    pos: usize,
    line: usize,
    col: usize,
    indent_stack: Vec<usize>,
    pending: Vec<Token>,
    at_line_start: bool,
    paren_level: usize,
    fstring_quote: Option<char>,
    fstring_parts: Vec<(String, String, String, u8)>, // (literal, expr_text, format_spec, conversion)
    fstring_part_idx: usize,
    fstring_expr_pending: Vec<Token>,
    /// Lexer-detected error pending emission (e.g. unindent does not match).
    pending_error: Option<String>,
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
            fstring_quote: None,
            fstring_parts: Vec::new(),
            fstring_part_idx: 0,
            fstring_expr_pending: Vec::new(),
            pending_error: None,
        }
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
        c.is_ascii_alphabetic() || c == '_' || c.is_alphabetic()
    }

    fn is_identifier_continue(c: char) -> bool {
        // XID_Continue approximates alphanumeric + combining marks (Mn/Mc)
        // + connector punctuation (Pc) + ZWNJ/ZWJ. Variation selectors
        // (U+FE00..U+FE0F, U+E0100..U+E01EF) are Mn — `x\U000e0100` is one
        // identifier in test_unicode_identifiers (CPython accepts it).
        c.is_ascii_alphanumeric()
            || c == '_'
            || c.is_alphanumeric()
            || matches!(
                c,
                '\u{0300}'..='\u{036F}'
                    | '\u{0591}'..='\u{05BD}'
                    | '\u{05BF}'
                    | '\u{05C1}'..='\u{05C2}'
                    | '\u{05C4}'..='\u{05C5}'
                    | '\u{0610}'..='\u{061A}'
                    | '\u{064B}'..='\u{065F}'
                    | '\u{0670}'
                    | '\u{06D6}'..='\u{06DC}'
                    | '\u{06DF}'..='\u{06E4}'
                    | '\u{06E7}'..='\u{06E8}'
                    | '\u{06EA}'..='\u{06ED}'
                    | '\u{0711}'
                    | '\u{0730}'..='\u{074A}'
                    | '\u{07A6}'..='\u{07B0}'
                    | '\u{07EB}'..='\u{07F3}'
                    | '\u{0816}'..='\u{0819}'
                    | '\u{081B}'..='\u{0823}'
                    | '\u{0825}'..='\u{0827}'
                    | '\u{0829}'..='\u{082D}'
                    | '\u{0859}'..='\u{085B}'
                    | '\u{0898}'..='\u{089F}'
                    | '\u{08CA}'..='\u{08E1}'
                    | '\u{08E3}'..='\u{0902}'
                    | '\u{093A}'
                    | '\u{093C}'
                    | '\u{0941}'..='\u{0948}'
                    | '\u{094D}'
                    | '\u{0951}'..='\u{0957}'
                    | '\u{0962}'..='\u{0963}'
                    | '\u{0981}'
                    | '\u{09BC}'
                    | '\u{09C1}'..='\u{09C4}'
                    | '\u{09CD}'
                    | '\u{0A3C}'
                    | '\u{0A41}'..='\u{0A42}'
                    | '\u{0A47}'..='\u{0A48}'
                    | '\u{0A4B}'..='\u{0A4D}'
                    | '\u{0A70}'..='\u{0A71}'
                    | '\u{0A81}'..='\u{0A82}'
                    | '\u{0ABC}'
                    | '\u{0AC1}'..='\u{0AC5}'
                    | '\u{0AC7}'..='\u{0AC8}'
                    | '\u{0ACD}'
                    | '\u{0AE2}'..='\u{0AE3}'
                    | '\u{0B01}'
                    | '\u{0B3C}'
                    | '\u{0B3F}'
                    | '\u{0B41}'..='\u{0B44}'
                    | '\u{0B4D}'
                    | '\u{0B55}'..='\u{0B56}'
                    | '\u{0B62}'..='\u{0B63}'
                    | '\u{0B82}'
                    | '\u{0BC0}'
                    | '\u{0BCD}'
                    | '\u{0C3C}'
                    | '\u{0C3E}'..='\u{0C40}'
                    | '\u{0C46}'..='\u{0C48}'
                    | '\u{0C4A}'..='\u{0C4D}'
                    | '\u{0C55}'..='\u{0C56}'
                    | '\u{0C62}'..='\u{0C63}'
                    | '\u{0CBC}'
                    | '\u{0CBF}'
                    | '\u{0CC6}'
                    | '\u{0CCC}'..='\u{0CCD}'
                    | '\u{0CE2}'..='\u{0CE3}'
                    | '\u{0D3B}'..='\u{0D3C}'
                    | '\u{0D41}'..='\u{0D44}'
                    | '\u{0D4D}'
                    | '\u{0D62}'..='\u{0D63}'
                    | '\u{0DCA}'
                    | '\u{0DD2}'..='\u{0DD4}'
                    | '\u{0DD6}'
                    | '\u{0E31}'
                    | '\u{0E34}'..='\u{0E3A}'
                    | '\u{0E47}'..='\u{0E4E}'
                    | '\u{0EB1}'
                    | '\u{0EB4}'..='\u{0EBC}'
                    | '\u{0EC8}'..='\u{0ECE}'
                    | '\u{0F18}'..='\u{0F19}'
                    | '\u{0F35}'
                    | '\u{0F37}'
                    | '\u{0F39}'
                    | '\u{0F71}'..='\u{0F7E}'
                    | '\u{0F80}'..='\u{0F84}'
                    | '\u{0F86}'..='\u{0F87}'
                    | '\u{0F8D}'..='\u{0F97}'
                    | '\u{0F99}'..='\u{0FBC}'
                    | '\u{0FC6}'
                    | '\u{102D}'..='\u{1030}'
                    | '\u{1032}'..='\u{1037}'
                    | '\u{1039}'..='\u{103A}'
                    | '\u{103D}'..='\u{103E}'
                    | '\u{1058}'..='\u{1059}'
                    | '\u{105E}'..='\u{1060}'
                    | '\u{1071}'..='\u{1074}'
                    | '\u{1082}'
                    | '\u{1085}'..='\u{1086}'
                    | '\u{108D}'
                    | '\u{109D}'
                    | '\u{135D}'..='\u{135F}'
                    | '\u{1712}'..='\u{1714}'
                    | '\u{1732}'..='\u{1733}'
                    | '\u{1752}'..='\u{1753}'
                    | '\u{1772}'..='\u{1773}'
                    | '\u{17B4}'..='\u{17B5}'
                    | '\u{17B7}'..='\u{17BD}'
                    | '\u{17C6}'
                    | '\u{17C9}'..='\u{17D3}'
                    | '\u{17DD}'
                    | '\u{180B}'..='\u{180D}'
                    | '\u{180F}'
                    | '\u{1885}'..='\u{1886}'
                    | '\u{18A9}'
                    | '\u{1920}'..='\u{1922}'
                    | '\u{1927}'..='\u{1928}'
                    | '\u{1932}'
                    | '\u{1939}'..='\u{193B}'
                    | '\u{1A17}'..='\u{1A18}'
                    | '\u{1A1B}'
                    | '\u{1A56}'
                    | '\u{1A58}'..='\u{1A5E}'
                    | '\u{1A60}'
                    | '\u{1A62}'
                    | '\u{1A65}'..='\u{1A6C}'
                    | '\u{1A73}'..='\u{1A7C}'
                    | '\u{1A7F}'
                    | '\u{1AB0}'..='\u{1ABE}'
                    | '\u{1B00}'..='\u{1B03}'
                    | '\u{1B34}'
                    | '\u{1B36}'..='\u{1B3A}'
                    | '\u{1B3C}'
                    | '\u{1B42}'
                    | '\u{1B6B}'..='\u{1B73}'
                    | '\u{1B80}'..='\u{1B81}'
                    | '\u{1BA2}'..='\u{1BA5}'
                    | '\u{1BA8}'..='\u{1BA9}'
                    | '\u{1BAB}'..='\u{1BAD}'
                    | '\u{1BE6}'
                    | '\u{1BE8}'..='\u{1BE9}'
                    | '\u{1BED}'
                    | '\u{1BEF}'..='\u{1BF1}'
                    | '\u{1C2C}'..='\u{1C33}'
                    | '\u{1C36}'..='\u{1C37}'
                    | '\u{1CD0}'..='\u{1CD2}'
                    | '\u{1CD4}'..='\u{1CE0}'
                    | '\u{1CE2}'..='\u{1CE8}'
                    | '\u{1CED}'
                    | '\u{1CF4}'
                    | '\u{1CF8}'..='\u{1CF9}'
                    | '\u{1DC0}'..='\u{1DFF}'
                    | '\u{200C}'..='\u{200D}'
                    | '\u{203F}'..='\u{2040}'
                    | '\u{2054}'
                    | '\u{20D0}'..='\u{20DC}'
                    | '\u{20E1}'
                    | '\u{20E5}'..='\u{20F0}'
                    | '\u{2CEF}'..='\u{2CF1}'
                    | '\u{2D7F}'
                    | '\u{2DE0}'..='\u{2DFF}'
                    | '\u{302A}'..='\u{302D}'
                    | '\u{3099}'..='\u{309A}'
                    | '\u{A66F}'
                    | '\u{A674}'..='\u{A67D}'
                    | '\u{A69E}'..='\u{A69F}'
                    | '\u{A6F0}'..='\u{A6F1}'
                    | '\u{A802}'
                    | '\u{A806}'
                    | '\u{A80B}'
                    | '\u{A825}'..='\u{A826}'
                    | '\u{A82C}'
                    | '\u{A8C4}'..='\u{A8C5}'
                    | '\u{A8E0}'..='\u{A8F1}'
                    | '\u{A8FF}'
                    | '\u{A926}'..='\u{A92D}'
                    | '\u{A947}'..='\u{A951}'
                    | '\u{A980}'..='\u{A982}'
                    | '\u{A9B3}'
                    | '\u{A9B6}'..='\u{A9B9}'
                    | '\u{A9BC}'
                    | '\u{A9BD}'..='\u{A9C0}'
                    | '\u{A9E5}'
                    | '\u{AA29}'..='\u{AA2E}'
                    | '\u{AA31}'..='\u{AA32}'
                    | '\u{AA35}'..='\u{AA36}'
                    | '\u{AA43}'
                    | '\u{AA4C}'
                    | '\u{AA7C}'
                    | '\u{AAB0}'
                    | '\u{AAB2}'..='\u{AAB4}'
                    | '\u{AAB7}'..='\u{AAB8}'
                    | '\u{AABE}'..='\u{AABF}'
                    | '\u{AAC1}'
                    | '\u{AAEC}'..='\u{AAED}'
                    | '\u{AAF6}'
                    | '\u{ABE5}'
                    | '\u{ABE8}'
                    | '\u{ABED}'
                    | '\u{FB1E}'
                    | '\u{FE00}'..='\u{FE0F}'
                    | '\u{FE20}'..='\u{FE2F}'
                    | '\u{FE33}'..='\u{FE34}'
                    | '\u{FE4D}'..='\u{FE4F}'
                    | '\u{FF3F}'
                    | '\u{FF9E}'..='\u{FF9F}'
                    | '\u{101FD}'
                    | '\u{102E0}'
                    | '\u{10376}'..='\u{1037A}'
                    | '\u{10A01}'..='\u{10A03}'
                    | '\u{10A05}'..='\u{10A06}'
                    | '\u{10A0C}'..='\u{10A0F}'
                    | '\u{10A38}'..='\u{10A3A}'
                    | '\u{10A3F}'
                    | '\u{10AE5}'..='\u{10AE6}'
                    | '\u{10D24}'..='\u{10D27}'
                    | '\u{10EAB}'..='\u{10EAC}'
                    | '\u{10EFD}'..='\u{10EFF}'
                    | '\u{10F46}'..='\u{10F50}'
                    | '\u{10F82}'..='\u{10F85}'
                    | '\u{11001}'
                    | '\u{11038}'..='\u{11046}'
                    | '\u{11070}'
                    | '\u{11073}'..='\u{11074}'
                    | '\u{1107F}'..='\u{11081}'
                    | '\u{110B3}'..='\u{110B6}'
                    | '\u{110B9}'..='\u{110BA}'
                    | '\u{110C2}'
                    | '\u{11100}'..='\u{11102}'
                    | '\u{11127}'..='\u{1112B}'
                    | '\u{1112D}'..='\u{11134}'
                    | '\u{11173}'
                    | '\u{11180}'..='\u{11181}'
                    | '\u{111B6}'..='\u{111BE}'
                    | '\u{111C9}'..='\u{111CC}'
                    | '\u{111CF}'
                    | '\u{1122F}'..='\u{11231}'
                    | '\u{11234}'
                    | '\u{11236}'..='\u{11237}'
                    | '\u{1123E}'
                    | '\u{11241}'
                    | '\u{112DF}'
                    | '\u{112E3}'..='\u{112EA}'
                    | '\u{11300}'..='\u{11301}'
                    | '\u{1133B}'..='\u{1133C}'
                    | '\u{11340}'
                    | '\u{11366}'..='\u{1136C}'
                    | '\u{11370}'..='\u{11374}'
                    | '\u{11438}'..='\u{1143F}'
                    | '\u{11442}'..='\u{11444}'
                    | '\u{11446}'
                    | '\u{1145E}'
                    | '\u{114B3}'..='\u{114B8}'
                    | '\u{114BA}'
                    | '\u{114BF}'..='\u{114C0}'
                    | '\u{114C2}'..='\u{114C3}'
                    | '\u{115B2}'..='\u{115B5}'
                    | '\u{115BC}'..='\u{115BD}'
                    | '\u{115BF}'..='\u{115C0}'
                    | '\u{115DC}'..='\u{115DD}'
                    | '\u{11633}'..='\u{1163A}'
                    | '\u{1163D}'
                    | '\u{1163F}'..='\u{11640}'
                    | '\u{116AB}'
                    | '\u{116AD}'
                    | '\u{116B0}'..='\u{116B5}'
                    | '\u{116B7}'
                    | '\u{1171D}'..='\u{1171F}'
                    | '\u{11722}'..='\u{11725}'
                    | '\u{11727}'..='\u{1172B}'
                    | '\u{1182F}'..='\u{11837}'
                    | '\u{11839}'..='\u{1183A}'
                    | '\u{1193B}'..='\u{1193C}'
                    | '\u{1193E}'
                    | '\u{11943}'
                    | '\u{119D4}'..='\u{119D7}'
                    | '\u{119DA}'..='\u{119DB}'
                    | '\u{119E0}'
                    | '\u{11A01}'..='\u{11A0A}'
                    | '\u{11A33}'..='\u{11A38}'
                    | '\u{11A3B}'..='\u{11A3E}'
                    | '\u{11A47}'
                    | '\u{11A51}'..='\u{11A56}'
                    | '\u{11A59}'..='\u{11A5B}'
                    | '\u{11A8A}'..='\u{11A96}'
                    | '\u{11A98}'..='\u{11A99}'
                    | '\u{11C30}'..='\u{11C36}'
                    | '\u{11C38}'..='\u{11C3D}'
                    | '\u{11C3F}'
                    | '\u{11C92}'..='\u{11CA7}'
                    | '\u{11CAA}'..='\u{11CB0}'
                    | '\u{11CB2}'..='\u{11CB3}'
                    | '\u{11CB5}'..='\u{11CB6}'
                    | '\u{11D31}'..='\u{11D36}'
                    | '\u{11D3A}'
                    | '\u{11D3C}'..='\u{11D3D}'
                    | '\u{11D3F}'..='\u{11D45}'
                    | '\u{11D47}'
                    | '\u{11D90}'..='\u{11D91}'
                    | '\u{11D95}'
                    | '\u{11D97}'
                    | '\u{11EF3}'..='\u{11EF4}'
                    | '\u{11F00}'..='\u{11F01}'
                    | '\u{11F36}'..='\u{11F3A}'
                    | '\u{11F40}'
                    | '\u{11F42}'
                    | '\u{13430}'..='\u{13438}'
                    | '\u{13440}'
                    | '\u{13447}'..='\u{13455}'
                    | '\u{16AF0}'..='\u{16AF4}'
                    | '\u{16B30}'..='\u{16B36}'
                    | '\u{16F4F}'
                    | '\u{16F8F}'..='\u{16F92}'
                    | '\u{16FE4}'
                    | '\u{1BC9D}'..='\u{1BC9E}'
                    | '\u{1CF00}'..='\u{1CF2D}'
                    | '\u{1CF30}'..='\u{1CF46}'
                    | '\u{1D165}'..='\u{1D169}'
                    | '\u{1D16D}'..='\u{1D172}'
                    | '\u{1D17B}'..='\u{1D182}'
                    | '\u{1D185}'..='\u{1D18B}'
                    | '\u{1D1AA}'..='\u{1D1AD}'
                    | '\u{1D242}'..='\u{1D244}'
                    | '\u{1DA00}'..='\u{1DA36}'
                    | '\u{1DA3B}'..='\u{1DA6C}'
                    | '\u{1DA75}'
                    | '\u{1DA84}'
                    | '\u{1DA9B}'..='\u{1DA9F}'
                    | '\u{1DAA1}'..='\u{1DAAF}'
                    | '\u{1E000}'..='\u{1E006}'
                    | '\u{1E008}'..='\u{1E018}'
                    | '\u{1E01B}'..='\u{1E021}'
                    | '\u{1E023}'..='\u{1E024}'
                    | '\u{1E026}'..='\u{1E02A}'
                    | '\u{1E08F}'
                    | '\u{1E130}'..='\u{1E136}'
                    | '\u{1E2AE}'
                    | '\u{1E2EC}'..='\u{1E2EF}'
                    | '\u{1E4EC}'..='\u{1E4EF}'
                    | '\u{1E8D0}'..='\u{1E8D6}'
                    | '\u{1E944}'..='\u{1E94A}'
                    | '\u{E0100}'..='\u{E01EF}'
            )
    }

    fn read_number(&mut self, first: char) -> Token {
        let mut s = String::new();
        s.push(first);

        if first == '0' {
            let next = self.peek();
            match next {
                Some('x' | 'X') => {
                    s.push(self.advance().unwrap());
                    while self
                        .peek()
                        .map_or(false, |c| Self::is_hex_char(c) || c == '_')
                    {
                        s.push(self.advance().unwrap());
                    }
                    return Token::Number(s);
                }
                Some('o' | 'O') => {
                    s.push(self.advance().unwrap());
                    while self
                        .peek()
                        .map_or(false, |c| Self::is_oct_char(c) || c == '_')
                    {
                        s.push(self.advance().unwrap());
                    }
                    return Token::Number(s);
                }
                Some('b' | 'B') => {
                    s.push(self.advance().unwrap());
                    while self
                        .peek()
                        .map_or(false, |c| Self::is_bin_char(c) || c == '_')
                    {
                        s.push(self.advance().unwrap());
                    }
                    return Token::Number(s);
                }
                _ => {}
            }
        }

        // A leading dot (`.995`, called with first=='.' from the main
        // dispatch loop below) is already a float — without this, the loop's
        // own `c == '.' && !is_float` check would still be false→false and
        // (wrongly) accept a second dot as if this weren't already one.
        let mut is_float = first == '.';
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

    fn read_bytes(&mut self, quote: char, raw: bool) -> Token {
        let mut bytes = Vec::new();
        let triple = self.peek() == Some(quote) && self.peek_ahead(1) == Some(quote);

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
        let triple = self.peek() == Some(quote) && self.peek_ahead(1) == Some(quote);

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
                    Some(c) if c == quote => break,
                    Some(c) => s.push(c),
                }
            }
        }

        Token::String(s)
    }

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
                        "match" => Token::Name("match".to_string()),
                        "case" => Token::Name("case".to_string()),
                        "_" => Token::Underscore,
                        _ => Token::Name(name),
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

                _ => return Token::Name(ch.to_string()),
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
                                    if state == 0 {
                                        depth += 1;
                                    }
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
                            break;
                        }
                        literal.push(c);
                    } else {
                        break;
                    }
                }
                Some(c) => literal.push(c),
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
