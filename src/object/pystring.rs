/// WTF-8 string type for Python `str` objects.
///
/// Python strings can contain lone surrogates (U+D800-U+DFFF) which are
/// invalid in UTF-8. This type stores strings in one of two encodings:
///
/// - **Utf8**: Valid UTF-8 (the common case, backed by `CompactString`)
/// - **Wtf8**: WTF-8 bytes that may contain surrogates, plus a cached
///   lossy UTF-8 version for `Deref<Target=str>`.
use compact_str::CompactString;

#[derive(Clone, Debug)]
pub enum PyStr {
    Utf8(CompactString),
    Wtf8 {
        raw: Vec<u8>,
        lossy: CompactString,
    },
}

impl PyStr {
    /// Create from a valid UTF-8 string (the common path).
    #[inline]
    pub fn from_str(s: &str) -> Self {
        PyStr::Utf8(CompactString::from(s))
    }

    /// Create from raw WTF-8 bytes that may contain surrogates.
    pub fn from_wtf8(bytes: Vec<u8>) -> Self {
        if let Ok(s) = std::str::from_utf8(&bytes) {
            PyStr::Utf8(CompactString::from(s))
        } else {
            let lossy = String::from_utf8_lossy(&bytes).into_owned();
            PyStr::Wtf8 {
                raw: bytes,
                lossy: CompactString::from(lossy),
            }
        }
    }

    /// Create a PyStr from a single surrogate codepoint (0xD800-0xDFFF).
    pub fn from_surrogate(cp: u32) -> Self {
        assert!(matches!(cp, 0xD800..=0xDFFF), "not a surrogate: U+{:04X}", cp);
        let b1 = 0xE0 | ((cp >> 12) as u8);
        let b2 = 0x80 | ((cp >> 6) as u8 & 0x3F);
        let b3 = 0x80 | (cp as u8 & 0x3F);
        let raw = vec![b1, b2, b3];
        let lossy = CompactString::from("\u{FFFD}");
        PyStr::Wtf8 { raw, lossy }
    }

    /// Borrow as `&str`. For WTF-8 strings, returns lossy UTF-8
    /// (surrogates replaced by U+FFFD). Safe for interning, hashing, etc.
    #[inline]
    pub fn as_str(&self) -> &str {
        match self {
            PyStr::Utf8(s) => s.as_str(),
            PyStr::Wtf8 { lossy, .. } => lossy.as_str(),
        }
    }

    /// Access raw WTF-8 bytes (valid for both Utf8 and Wtf8 variants).
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            PyStr::Utf8(s) => s.as_bytes(),
            PyStr::Wtf8 { raw, .. } => raw.as_slice(),
        }
    }

    /// Byte length.
    #[inline]
    pub fn len(&self) -> usize {
        match self {
            PyStr::Utf8(s) => s.len(),
            PyStr::Wtf8 { raw, .. } => raw.len(),
        }
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Convert to owned `String`, replacing surrogates with U+FFFD.
    #[inline]
    pub fn to_string_lossy(&self) -> String {
        match self {
            PyStr::Utf8(s) => s.to_string(),
            PyStr::Wtf8 { lossy, .. } => lossy.to_string(),
        }
    }

    /// Whether this string contains lone surrogates (WTF-8 variant).
    #[inline]
    pub fn has_surrogates(&self) -> bool {
        matches!(self, PyStr::Wtf8 { .. })
    }

    /// Repeat the string `n` times, preserving surrogates.
    pub fn repeat(&self, n: usize) -> PyStr {
        if self.has_surrogates() {
            let mut bytes = Vec::with_capacity(self.len() * n);
            for _ in 0..n {
                bytes.extend_from_slice(self.as_bytes());
            }
            PyStr::from_wtf8(bytes)
        } else {
            PyStr::Utf8(CompactString::from(self.as_str().repeat(n)))
        }
    }

    /// Lines iterator (delegates to str::lines via Deref).
    pub fn lines(&self) -> std::str::Lines<'_> {
        self.as_str().lines()
    }

    /// In-place reserve for the Utf8 variant (used by fast str +=).
    /// Returns false if this is a Wtf8 string (caller should use copy path).
    pub fn reserve(&mut self, additional: usize) -> bool {
        match self {
            PyStr::Utf8(s) => {
                s.reserve(additional);
                true
            }
            PyStr::Wtf8 { .. } => false,
        }
    }

    /// In-place push_str for the Utf8 variant.
    /// Returns false if this is a Wtf8 string.
    pub fn push_str_inplace(&mut self, s: &str) -> bool {
        match self {
            PyStr::Utf8(cs) => {
                cs.push_str(s);
                true
            }
            PyStr::Wtf8 { .. } => false,
        }
    }

    /// Concatenate two PyStrs, preserving surrogates in WTF-8.
    pub fn concat_wtf8(a: &PyStr, b: &PyStr) -> PyStr {
        let mut bytes = Vec::with_capacity(a.len() + b.len());
        bytes.extend_from_slice(a.as_bytes());
        bytes.extend_from_slice(b.as_bytes());
        PyStr::from_wtf8(bytes)
    }
}

// Deref to str so &PyStr can be used as &str in most contexts.
// For Wtf8, this returns the lossy cached string (surrogates → U+FFFD).
impl std::ops::Deref for PyStr {
    type Target = str;
    #[inline]
    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl std::fmt::Display for PyStr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl PartialEq for PyStr {
    fn eq(&self, other: &Self) -> bool {
        self.as_bytes() == other.as_bytes()
    }
}

impl Eq for PyStr {}

impl PartialEq<str> for PyStr {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl PartialEq<&str> for PyStr {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl PartialOrd for PyStr {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PyStr {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.as_bytes().cmp(other.as_bytes())
    }
}

impl std::hash::Hash for PyStr {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.as_bytes().hash(state)
    }
}

impl Default for PyStr {
    fn default() -> Self {
        PyStr::Utf8(CompactString::new(""))
    }
}

impl From<&str> for PyStr {
    fn from(s: &str) -> Self {
        PyStr::Utf8(CompactString::from(s))
    }
}

impl From<String> for PyStr {
    fn from(s: String) -> Self {
        PyStr::Utf8(CompactString::from(s))
    }
}

impl From<&String> for PyStr {
    fn from(s: &String) -> Self {
        PyStr::Utf8(CompactString::from(s.as_str()))
    }
}

/// Iterator over WTF-8 codepoints (yields u32 including surrogates).
pub struct Wtf8Chars<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Wtf8Chars<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Wtf8Chars { bytes, pos: 0 }
    }
}

impl Iterator for Wtf8Chars<'_> {
    type Item = u32;

    fn next(&mut self) -> Option<u32> {
        if self.pos >= self.bytes.len() {
            return None;
        }
        let b0 = self.bytes[self.pos];
        let (cp, advance) = if b0 < 0x80 {
            (b0 as u32, 1)
        } else if b0 < 0xE0 {
            let b1 = self.bytes.get(self.pos + 1).copied().unwrap_or(0);
            let cp = ((b0 as u32 & 0x1F) << 6) | (b1 as u32 & 0x3F);
            (cp, 2)
        } else if b0 < 0xF0 {
            let b1 = self.bytes.get(self.pos + 1).copied().unwrap_or(0);
            let b2 = self.bytes.get(self.pos + 2).copied().unwrap_or(0);
            let cp = ((b0 as u32 & 0x0F) << 12) | ((b1 as u32 & 0x3F) << 6) | (b2 as u32 & 0x3F);
            (cp, 3)
        } else {
            let b1 = self.bytes.get(self.pos + 1).copied().unwrap_or(0);
            let b2 = self.bytes.get(self.pos + 2).copied().unwrap_or(0);
            let b3 = self.bytes.get(self.pos + 3).copied().unwrap_or(0);
            let cp = ((b0 as u32 & 0x07) << 18)
                | ((b1 as u32 & 0x3F) << 12)
                | ((b2 as u32 & 0x3F) << 6)
                | (b3 as u32 & 0x3F);
            (cp, 4)
        };
        self.pos += advance;
        Some(cp)
    }
}
