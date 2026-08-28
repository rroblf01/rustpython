use std::fmt;
use unicode_normalization::UnicodeNormalization;

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
}

mod core;
mod strings;
