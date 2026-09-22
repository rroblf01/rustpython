use crate::object::*;
use std::collections::HashMap;

pub fn re_pattern_error(msg: String, pattern: Option<String>, pos: Option<i64>) -> PyError {
    // CPython's PatternError: `.msg` is the bare message; `str(err)` is
    // `"{msg} at position {pos}"`; `.lineno`/`.colno` locate `pos` (a CHAR
    // index into the pattern) on its line. (test_error asserts all of these.)
    let mut extra = HashMap::new();
    extra.insert("msg".to_string(), py_str(&msg));
    extra.insert(
        "pattern".to_string(),
        match &pattern {
            Some(p) => py_str(p),
            None => py_none(),
        },
    );
    extra.insert(
        "pos".to_string(),
        match pos {
            Some(p) => py_int(p),
            None => py_none(),
        },
    );
    if let (Some(p), Some(pat)) = (pos, &pattern) {
        let p_usize = (p as usize).min(pat.chars().count());
        let mut lineno = 1i64;
        let mut colno = 1i64;
        for (i, ch) in pat.chars().enumerate() {
            if i >= p_usize {
                break;
            }
            if ch == '\n' {
                lineno += 1;
                colno = 1;
            } else {
                colno += 1;
            }
        }
        extra.insert("lineno".to_string(), py_int(lineno));
        extra.insert("colno".to_string(), py_int(colno));
    }
    let display = match pos {
        Some(p) => format!("{} at position {}", msg, p),
        None => msg.clone(),
    };
    let exc = PyObjectRef::new(PyObject::Exception {
        typ: "PatternError".to_string(),
        args: vec![py_str(&display)],
        cause: None,
        suppress_context: false,
        context: None,
        traceback: None,
        extra: Some(extra),
    });
    PyError::Exception("PatternError".to_string(), exc)
}

/// Python's `re` treats a `{` that doesn't form a valid `{n}`/`{n,}`/`{n,m}`
/// counted-repetition quantifier as a literal character; Rust's `regex`
/// crate instead rejects it as a parse error ("repetition operator missing
/// expression"). Real-world patterns lean on this leniency constantly
/// (e.g. Django's template-tag detector `{%.*?%}`), so translate patterns
/// through this before compiling rather than surfacing the raw Rust error.
pub fn escape_loose_braces(pattern: &str) -> String {
    let chars: Vec<char> = pattern.chars().collect();
    let mut result = String::with_capacity(pattern.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' && i + 1 < chars.len() {
            result.push(c);
            result.push(chars[i + 1]);
            i += 2;
            continue;
        }
        if c == '{' {
            let mut j = i + 1;
            let mut saw_digit = false;
            while j < chars.len() && chars[j].is_ascii_digit() {
                j += 1;
                saw_digit = true;
            }
            if j < chars.len() && chars[j] == ',' {
                j += 1;
                while j < chars.len() && chars[j].is_ascii_digit() {
                    j += 1;
                }
            }
            if saw_digit && j < chars.len() && chars[j] == '}' {
                result.extend(&chars[i..=j]);
                i = j + 1;
            } else {
                result.push_str("\\{");
                i += 1;
            }
            continue;
        }
        result.push(c);
        i += 1;
    }
    result
}

/// Two related Python-`re`-vs-Rust-`regex` character-class gaps in one
/// pass, both hit by real code (CPython's own `email.utils.specialsre`,
/// `r'[][\\()<>@,:;".]'`):
///
/// 1. A `]` right after the opening `[` (or `[^`) of a class is a literal
///    `]`, not the closing bracket — Rust's `regex` crate actually *does*
///    already support this one natively (confirmed: `[]]`/`[]x]` compile
///    fine as-is) — no translation needed for this part by itself.
/// 2. A bare `[` appearing *inside* an already-open class (a plain literal
///    character there in Python/POSIX/PCRE — classes don't nest) is
///    mistaken by Rust's `regex` crate for the start of a *nested* class
///    it doesn't support, failing with "unclosed character class" the
///    moment the class also contains an unescaped `]` later (confirmed:
///    `[]x]` alone is fine, but `[][x]` and `[][\\()<>@,:;".]` both fail;
///    `[]\[]`, with the inner `[` pre-escaped, works). This is the part
///    that actually needs translating — every bare `[` found while already
///    inside a class gets escaped to `\[`.
///
/// Both are handled by the same single-pass `in_class` scan below (the
/// leading-`]` case doesn't need output changes, just correct state
/// tracking so the following bare-`[` fix doesn't misfire on it). The same
/// pass also translates octal character escapes (`\NNN`) to `\x{...}` when
/// inside a class — see the comment at that branch for why it's scoped to
/// in-class only.
pub fn escape_leading_bracket_in_class(pattern: &str) -> String {
    let chars: Vec<char> = pattern.chars().collect();
    let mut result = String::with_capacity(pattern.len());
    let mut i = 0;
    let mut in_class = false;
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' && i + 1 < chars.len() {
            // Python's `re` accepts `\NNN` (1-3 octal digits) as an octal
            // character escape (real code: CPython's own `email.header`,
            // `r'[\041-\176]+:$'`, an ASCII-printable-range class). Rust's
            // `regex` crate has no octal-escape syntax and reads a
            // backslash-digit sequence as a *backreference* attempt
            // instead — which it doesn't support at all — rejecting it
            // outright. Only translate this inside a character class,
            // where a backreference could never be valid syntax in any
            // regex flavor anyway (so there's no ambiguity to worry about,
            // unlike outside a class where `\1` etc. legitimately mean
            // "backreference to group 1" in real patterns elsewhere).
            if in_class && chars[i + 1].is_digit(8) {
                let mut j = i + 1;
                let mut value: u32 = 0;
                let mut digits = 0;
                while j < chars.len() && digits < 3 && chars[j].is_digit(8) {
                    value = value * 8 + chars[j].to_digit(8).unwrap();
                    j += 1;
                    digits += 1;
                }
                result.push_str(&format!("\\x{{{:x}}}", value));
                i = j;
                continue;
            }
            result.push(c);
            result.push(chars[i + 1]);
            i += 2;
            continue;
        }
        if !in_class && c == '[' {
            result.push(c);
            i += 1;
            in_class = true;
            if i < chars.len() && chars[i] == '^' {
                result.push('^');
                i += 1;
            }
            // A `]` right here (the very first character of the class,
            // after an optional `^`) is a literal `]`, not the closing
            // bracket — Rust's `regex` crate needs it spelled `\]` to
            // agree; every subsequent `[`/`]` until the *real* close is
            // handled by the `in_class` tracking below instead of this
            // one-shot check, so a second literal `[` right after (like
            // `r'[][...]'`, `]` then `[` both literal) is never mistaken
            // for the start of a nested class.
            if i < chars.len() && chars[i] == ']' {
                result.push('\\');
                result.push(']');
                i += 1;
            }
            continue;
        }
        if in_class && c == ']' {
            in_class = false;
            result.push(c);
            i += 1;
            continue;
        }
        if in_class && c == '[' {
            // A bare `[` here is just a literal character in Python/POSIX
            // (classes don't nest) — Rust's `regex` crate reads it as
            // attempting a nested class instead, so escape it.
            result.push('\\');
            result.push('[');
            i += 1;
            continue;
        }
        result.push(c);
        i += 1;
    }
    result
}

/// Python's `re.sub`/`Pattern.sub` replacement strings reference capture
/// groups as `\1`, `\g<1>`, `\g<name>` — the `regex`/`fancy_regex` crates'
/// `Replacer` impl for `&str` instead uses Perl/sed-style `$1`/`${1}`/`${name}`
/// and treats a literal `$` specially. Translate before calling
/// `replace_all`/`replace`, or every `\N`-backreference replacement (an
/// extremely common idiom — e.g. Django's own `camel_case_to_spaces`:
/// `re_camel_case.sub(r" \1", value)`) silently emits the backreference
/// syntax itself instead of the captured text.
pub fn count_capturing_groups(pattern: &str) -> usize {
    let chars: Vec<char> = pattern.chars().collect();
    let mut count = 0usize;
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\\' && i + 1 < chars.len() {
            i += 2;
            continue;
        }
        if chars[i] == '[' {
            i += 1;
            while i < chars.len() && chars[i] != ']' {
                if chars[i] == '\\' && i + 1 < chars.len() {
                    i += 2;
                } else {
                    i += 1;
                }
            }
            if i < chars.len() {
                i += 1;
            }
            continue;
        }
        if chars[i] == '(' {
            if i + 1 < chars.len() && chars[i + 1] == '?' {
                if i + 2 < chars.len() && chars[i + 2] == '(' {
                    // Conditional `(?(` — the inner `(` is part of the syntax,
                    // not a capturing group; skip past `(?(` so it isn't counted.
                    i += 3;
                    continue;
                }
                if i + 3 < chars.len() && chars[i + 2] == 'P' && chars[i + 3] == '<' {
                    count += 1;
                }
            } else {
                count += 1;
            }
        }
        i += 1;
    }
    count
}

pub fn translate_pattern_backrefs_and_octal(pattern: &str) -> String {
    let num_groups = count_capturing_groups(pattern);
    let chars: Vec<char> = pattern.chars().collect();
    let mut out = String::with_capacity(pattern.len());
    let mut i = 0;
    let mut in_class = false;
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' && i + 1 < chars.len() {
            let nxt = chars[i + 1];
            if nxt.is_ascii_digit() && !in_class {
                let mut j = i + 1;
                while j < chars.len() && chars[j].is_ascii_digit() {
                    j += 1;
                }
                let digits: String = chars[i + 1..j].iter().collect();
                if digits.starts_with('0') {
                    let mut k = i + 1;
                    let mut oct_digits = String::new();
                    let mut cnt = 0;
                    while k < chars.len() && cnt < 3 && chars[k].is_digit(8) {
                        oct_digits.push(chars[k]);
                        k += 1;
                        cnt += 1;
                    }
                    if !oct_digits.is_empty() {
                        if let Ok(val) = u32::from_str_radix(&oct_digits, 8) {
                            if val <= 0o377 {
                                out.push_str(&format!("\\x{{{:x}}}", val));
                                for idx in k..j {
                                    out.push(chars[idx]);
                                }
                                i = j;
                                continue;
                            }
                        }
                    }
                    out.push(c);
                    out.push(nxt);
                    i += 2;
                    continue;
                }
                // CPython: `\` + nonzero digit is an octal escape when the
                // FIRST THREE digits are octal (value ≤ 0o377) — even if more
                // digits follow (`\1000` → octal `\100` + literal `0`); it is
                // a decimal group reference (max TWO digits) otherwise.
                let d: Vec<char> = digits.chars().collect();
                if d.len() >= 3 && d[0].is_digit(8) && d[1].is_digit(8) && d[2].is_digit(8) {
                    let first3: String = d[..3].iter().collect();
                    if let Ok(val) = u32::from_str_radix(&first3, 8) {
                        if val <= 0o377 {
                            out.push_str(&format!("\\x{{{:x}}}", val));
                            for idx in (i + 1 + 3)..j {
                                out.push(chars[idx]); // leftover digits literal
                            }
                            i = j;
                            continue;
                        }
                        // three octal digits but value > 0o377 → leave for the
                        // engine's "octal escape value ... outside of range" error.
                        out.push(c);
                        out.extend(d.iter().take(3));
                        for idx in (i + 1 + 3)..j {
                            out.push(chars[idx]);
                        }
                        i = j;
                        continue;
                    }
                }
                // Group reference (original prefix-matching behavior restored):
                let digits_all: String = d.iter().collect();
                let num: usize = digits_all.parse().unwrap_or(0);
                if num != 0 && num <= num_groups {
                    out.push(c);
                    out.extend(d.iter());
                    i = j;
                    continue;
                } else if num > num_groups && num_groups > 0 {
                    // Overflowing backref: find the longest valid group prefix
                    // and emit `\g<prefix>` + leftover digits (mirrors prior
                    // behavior so `\119` with12 groups → backref11 + `9`).
                    let mut found = None;
                    for len in (1..d.len()).rev() {
                        if let Ok(prefix) = digits_all[..len].parse::<usize>() {
                            if prefix != 0 && prefix <= num_groups {
                                found = Some(len);
                                break;
                            }
                        }
                    }
                    if let Some(len) = found {
                        out.push_str(&format!("\\g<{}>", &digits_all[..len]));
                        out.extend(digits_all[len..].chars());
                        i = j;
                        continue;
                    }
                }
                out.push(c);
                out.extend(d.iter());
                i = j;
                continue;
            } else {
                out.push(c);
                if i + 1 < chars.len() {
                    out.push(chars[i + 1]);
                    i += 2;
                } else {
                    i += 1;
                }
                continue;
            }
        }
        if !in_class && c == '[' {
            in_class = true;
        } else if in_class && c == ']' {
            in_class = false;
        }
        out.push(c);
        i += 1;
    }
    out
}

/// Translate a Python replacement string into a regex-engine template.
/// Group references `\N`/`\g<name>` → `${N}`/`${name}`, octal escapes
/// → literal chars, `\n`/`\t`/etc. → control chars, `$$` ← `$`.
/// Bad escapes (`\q`, `\x41`) raise a `re.PatternError` matching
/// CPython's `re._parse_template` error messages.
pub fn translate_python_replacement(repl: &str) -> PyResult<String> {
    let chars: Vec<char> = repl.chars().collect();
    let mut out = String::with_capacity(repl.len());
    let mut i = 0;
    let is_oct = |c: char| matches!(c, '0'..='7');
    while i < chars.len() {
        let c = chars[i];
        if c == '$' {
            out.push_str("$$");
            i += 1;
        } else if c == '\\' && i + 1 < chars.len() {
            let next = chars[i + 1];
            match next {
                'g' => {
                    // \g<name> → ${name}
                    if chars.get(i + 2) == Some(&'<') {
                        let mut j = i + 3;
                        while j < chars.len() && chars[j] != '>' {
                            j += 1;
                        }
                        out.push_str("${");
                        out.extend(&chars[i + 3..j]);
                        out.push('}');
                        i = if j < chars.len() { j + 1 } else { j };
                    } else {
                        // \g without < — validate_g_template catches it later
                        out.push_str("\\g");
                        i += 2;
                    }
                }
                '\\' => {
                    out.push('\\');
                    i += 2;
                }
                '0' => {
                    // Octal escape: \0 + up to 2 more octal digits, masked
                    // to 0xff to mirror CPython's template octal encoding.
                    let mut digits = String::from("0");
                    let mut j = i + 2;
                    while digits.len() < 3 && j < chars.len() && is_oct(chars[j]) {
                        digits.push(chars[j]);
                        j += 1;
                    }
                    let v = u32::from_str_radix(&digits, 8).unwrap_or(0) & 0xff;
                    out.push(char::from_u32(v).unwrap_or('\u{FFFD}'));
                    i = j;
                }
                '1'..='9' => {
                    // CPython template octal-vs-groupref disambiguation:
                    // `\N` is a *two-digit* decimal group reference unless
                    // three consecutive 0-7 digits form an octal escape.
                    let d1 = next;
                    let d2 = chars.get(i + 2).copied();
                    if let Some(d2) = d2 {
                        if d2.is_ascii_digit() {
                            let d3 = chars.get(i + 3).copied();
                            if is_oct(d1) && is_oct(d2) && matches!(d3, Some(d) if is_oct(d)) {
                                let d3u = d3.unwrap();
                                let v = (d1.to_digit(8).unwrap() << 6)
                                    | (d2.to_digit(8).unwrap() << 3)
                                    | d3u.to_digit(8).unwrap();
                                if v > 0o377 {
                                    return Err(re_pattern_error(
                                        format!(
                                            "octal escape value \\{}{}{} outside of range 0-0o377",
                                            d1, d2, d3u
                                        ),
                                        Some(repl.to_string()),
                                        Some(0),
                                    ));
                                }
                                out.push(char::from_u32(v).unwrap_or('\u{FFFD}'));
                                i += 4;
                            } else {
                                // Two-digit group reference
                                out.push_str("${");
                                out.push(d1);
                                out.push(d2);
                                out.push('}');
                                i += 3;
                            }
                        } else {
                            // Single-digit group reference
                            out.push_str("${");
                            out.push(d1);
                            out.push('}');
                            i += 2;
                        }
                    } else {
                        // `\N` at end of template
                        out.push_str("${");
                        out.push(d1);
                        out.push('}');
                        i += 2;
                    }
                }
                'a' => { out.push('\x07'); i += 2; }
                'b' => { out.push('\x08'); i += 2; }
                'f' => { out.push('\x0C'); i += 2; }
                'n' => { out.push('\n'); i += 2; }
                'r' => { out.push('\r'); i += 2; }
                't' => { out.push('\t'); i += 2; }
                'v' => { out.push('\x0B'); i += 2; }
                ch if ch.is_ascii_alphabetic() => {
                    // Bad escape — mirrors CPython's _parse_template.
                    return Err(re_pattern_error(
                        format!("bad escape \\{}", ch),
                        Some(repl.to_string()),
                        Some((i) as i64),
                    ));
                }
                _ => {
                    // Unknown non-letter escape: keep verbatim
                    out.push('\\');
                    out.push(next);
                    i += 2;
                }
            }
        } else {
            out.push(c);
            i += 1;
        }
    }
    Ok(out)
}

pub fn validate_g_template(repl: &str, re: &fancy_regex::Regex, is_bytes: bool) -> PyResult<()> {
    // In bytes mode, non-ASCII bytes in group names render as `\xNN`
    // escapes in error messages (mirrors CPython).
    let disp = |name: &str| -> String {
        if !is_bytes {
            return name.to_string();
        }
        let mut out = String::new();
        for c in name.chars() {
            if (c as u32) < 128 {
                out.push(c);
            } else {
                // latin-1 roundtrip: the char value IS the raw byte.
                out.push_str(&format!("\\x{:02x}", c as u32));
            }
        }
        out
    };
    let chars: Vec<char> = repl.chars().collect();
    let mut i = 0;
    let n_groups = re.capture_names().count().saturating_sub(1);
    while i < chars.len() {
        if chars[i] == '\\' && i + 1 < chars.len() {
            let nxt = chars[i + 1];
            if nxt == 'g' {
                if i + 2 >= chars.len() || chars[i + 2] != '<' {
                    return Err(re_pattern_error(
                        "missing <".to_string(),
                        Some(repl.to_string()),
                        Some(2),
                    ));
                }
                let mut j = i + 3;
                while j < chars.len() && chars[j] != '>' {
                    j += 1;
                }
                let name: String = chars[i + 3..j].iter().collect();
                if j >= chars.len() && !name.is_empty() {
                    return Err(re_pattern_error(
                        "missing >, unterminated name".to_string(),
                        Some(repl.to_string()),
                        Some(3),
                    ));
                }
                if name.is_empty() {
                    return Err(re_pattern_error(
                        "missing group name".to_string(),
                        Some(repl.to_string()),
                        Some(3),
                    ));
                }
                let mut bad = false;
                for (idx, ch) in name.chars().enumerate() {
                    if idx == 0 && ch.is_ascii_digit() {
                        if let Ok(num) = name.parse::<usize>() {
                            // `\g<0>` = the whole match (valid).
                            if num > n_groups {
                                return Err(re_pattern_error(
                                    format!("invalid group reference {}", num),
                                    Some(repl.to_string()),
                                    Some(3),
                                ));
                            }
                        } else {
                            bad = true;
                        }
                        break;
                    }
                    // Group names follow identifier rules (mirrors
                    // CPython: `\g<µ>` works, `\g<㊀>`/`\g<¹>` are "bad
                    // character"). Bytes templates are ASCII-only.
                    let ok = if is_bytes {
                        ch.is_ascii_alphanumeric() || ch == '_'
                    } else if idx == 0 {
                        crate::token::Lexer::is_identifier_start(ch)
                            && !ch.is_ascii_digit()
                    } else {
                        crate::token::Lexer::is_identifier_continue(ch)
                    };
                    if !ok {
                        bad = true;
                        break;
                    }
                }
                if name.contains(' ') || name.contains('<') || name.contains('>') {
                    bad = true;
                }
                if bad {
                    return Err(re_pattern_error(
                        format!("bad character in group name '{}'", disp(&name)),
                        Some(repl.to_string()),
                        Some(3),
                    ));
                }
                i = j + 1;
                continue;
            } else if nxt.is_ascii_digit() {
                let is_oct_d = |c: char| matches!(c, '0'..='7');
                if nxt == '0' {
                    // octal literal: \0 + up to 2 extra octal digits
                    let mut j = i + 2;
                    let mut cnt = 0;
                    while j < chars.len() && is_oct_d(chars[j]) && cnt < 2 {
                        j += 1;
                        cnt += 1;
                    }
                    i = j;
                    continue;
                }
                // \1..\9: octal iff three consecutive 0-7 digits, which may
                // exceed 0o377 (range error) — otherwise a group reference.
                if i + 3 < chars.len()
                    && is_oct_d(chars[i + 1])
                    && is_oct_d(chars[i + 2])
                    && is_oct_d(chars[i + 3])
                {
                    let v = (chars[i + 1].to_digit(8).unwrap() << 6)
                        | (chars[i + 2].to_digit(8).unwrap() << 3)
                        | chars[i + 3].to_digit(8).unwrap();
                    if v > 0o377 {
                        return Err(re_pattern_error(
                            format!(
                                "octal escape value \\{}{}{} outside of range 0-0o377",
                                chars[i + 1], chars[i + 2], chars[i + 3]
                            ),
                            Some(repl.to_string()),
                            Some(0),
                        ));
                    }
                    i += 4;
                    continue;
                }
                // Group reference: one or two digits (never more).
                let mut j = i + 1;
                if i + 2 < chars.len() && chars[i + 2].is_ascii_digit() {
                    j = i + 2;
                }
                let num_str: String = chars[i + 1..j + 1].iter().collect();
                if let Ok(num) = num_str.parse::<usize>() {
                    if num == 0 || num > n_groups {
                        return Err(re_pattern_error(
                            format!("invalid group reference {}", num),
                            Some(repl.to_string()),
                            Some((i + 1) as i64),
                        ));
                    }
                }
                i = j + 1;
                continue;
            }
        }
        i += 1;
    }
    Ok(())
}

pub fn map_regex_error(msg: &str) -> String {
    let lower = msg.to_lowercase();
    if lower.contains("invalid back reference") {
        if let Some(pos) = lower.find("group") {
            let num_part = msg[pos + 5..].trim();
            let num: String = num_part.chars().filter(|c| c.is_ascii_digit()).collect();
            if !num.is_empty() {
                return format!("invalid group reference {}", num);
            }
        }
        return "invalid group reference".to_string();
    }
    if lower.contains("unclosed character class") || lower.contains("unterminated character set") {
        return "unterminated character set".to_string();
    }
    if lower.contains("unmatched") && lower.contains("parenthesis") {
        if lower.contains("unclosed") {
            return "missing ), unterminated subpattern".to_string();
        }
        return "unbalanced parenthesis".to_string();
    }
    if lower.contains("nothing to repeat") || lower.contains("repetition operator missing") {
        return "nothing to repeat".to_string();
    }
    msg.to_string()
}

/// Convert a `fancy_regex::Error` into a CPython-style `PatternError`,
/// extracting the error POSITION (so `.pos` is populated) and mapping the
/// error KIND to CPython's exact message. The plain
/// `map_regex_error(&e.to_string())` path lost the position (always
/// `.pos is None`) and didn't recognize fancy's own phrasing, which broke
/// `test_re`'s `checkPatternError(..., pos=N)` assertions.
pub fn fancy_error_to_pattern(e: &fancy_regex::Error, pattern: &str) -> PyError {
    match e {
        fancy_regex::Error::ParseError(pos, kind) => {
            // fancy reports a BYTE offset into the pattern; CPython's
            // `PatternError.pos` is a CHAR index (differs once the pattern
            // holds any multi-byte char, e.g. `'(€))'` → pos 3, not 5).
            let byte_pos = (*pos).min(pattern.len());
            let char_pos = pattern[..byte_pos].chars().count() as i64;
            let msg = map_parse_error(kind, pattern, char_pos as usize);
            re_pattern_error(msg, Some(pattern.to_string()), Some(char_pos))
        }
        fancy_regex::Error::CompileError(ce) => {
            // Look-behind / compile-stage errors: keep prior mapping but
            // surface a CPython message.
            let msg = map_compile_error(ce);
            re_pattern_error(msg, Some(pattern.to_string()), None)
        }
        _ => re_pattern_error(
            map_regex_error(&e.to_string()),
            Some(pattern.to_string()),
            None,
        ),
    }
}

fn map_compile_error(ce: &fancy_regex::CompileError) -> String {
    use fancy_regex::CompileError;
    match ce {
        CompileError::LookBehindNotConst => "look-behind requires fixed-width pattern".to_string(),
        CompileError::InvalidBackref(n) => format!("invalid group reference {}", n),
        CompileError::InvalidGroupName => "missing group name".to_string(),
        CompileError::SubroutineCallTargetNotFound(s, _) => {
            // fancy's message for `\g<N>` with a bad N (raised as a subroutine
            // call) — CPython reports this as "invalid group reference N".
            let num: String = s
                .chars()
                .rev()
                .take_while(|c| c.is_ascii_digit())
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            if !num.is_empty() {
                format!("invalid group reference {}", num)
            } else {
                "invalid group reference".to_string()
            }
        }
        _ => map_regex_error(&ce.to_string()),
    }
}

fn map_parse_error(kind: &fancy_regex::ParseError, pattern: &str, pos: usize) -> String {
    use fancy_regex::ParseError;
    match kind {
        // `TargetNotRepeatable` covers both "nothing to repeat" (`*` with no
        // target) and "multiple repeat" (`x**`). Classify by the char just
        // before the offending quantifier: if it is itself a quantifier
        // suffix (`*+?}`), the target was already quantified → multiple.
        ParseError::TargetNotRepeatable => {
            let chars: Vec<char> = pattern.chars().collect();
            let prev = if pos > 0 { chars.get(pos - 1).copied() } else { None };
            if matches!(prev, Some('*') | Some('+') | Some('?') | Some('}')) {
                "multiple repeat".to_string()
            } else {
                "nothing to repeat".to_string()
            }
        }
        ParseError::InvalidRepeat => "nothing to repeat".to_string(),
        ParseError::UnclosedOpenParen => "missing ), unterminated subpattern".to_string(),
        ParseError::TrailingBackslash => "bad escape (end of pattern)".to_string(),
        ParseError::InvalidEscape(s) => format!("bad escape {}", s),
        ParseError::InvalidBackref => "invalid group reference".to_string(),
        ParseError::InvalidGroupName => "missing group name".to_string(),
        ParseError::UnclosedUnicodeName => "missing ), unterminated name".to_string(),
        ParseError::InvalidClass => "unterminated character set".to_string(),
        ParseError::InvalidHex => "bad escape".to_string(),
        ParseError::GeneralParseError(s) => map_general_parse_error(s),
        _ => map_regex_error(&kind.to_string()),
    }
}

fn map_general_parse_error(s: &str) -> String {
    let lower = s.to_lowercase();
    if lower.contains("subroutine call") && lower.contains("non-existent group") {
        // fancy's message for `\g<N>` with a bad N — CPython calls this
        // "invalid group reference N".
        let num: String = s.chars().rev().take_while(|c| c.is_ascii_digit()).collect();
        let num: String = num.chars().rev().collect();
        if !num.is_empty() {
            return format!("invalid group reference {}", num);
        }
        return "invalid group reference".to_string();
    }
    if lower.contains("invalid back reference") || lower.contains("invalid group reference") {
        // Extract trailing group number if present.
        let num: String = s.chars().filter(|c| c.is_ascii_digit()).collect();
        if !num.is_empty() && lower.contains("group") {
            return format!("invalid group reference {}", num);
        }
        return "invalid group reference".to_string();
    }
    if lower.contains("nothing to repeat") {
        return "nothing to repeat".to_string();
    }
    if lower.contains("unterminated") && lower.contains("class") {
        return "unterminated character set".to_string();
    }
    if lower.contains("unterminated") || lower.contains("unclosed") {
        if lower.contains("name") {
            return "missing ), unterminated name".to_string();
        }
        return "missing ), unterminated subpattern".to_string();
    }
    s.to_string()
}

pub fn compile_python_regex(pattern: &str) -> Result<fancy_regex::Regex, fancy_regex::Error> {
    compile_python_regex_flags(pattern, 0, false)
}

/// Single inline-flag letter to its bit (mirrors CPython's `FLAGS`).
fn inline_flag_bit(c: char) -> Option<i32> {
    match c {
        'i' => Some(2),
        'L' => Some(4),
        'm' => Some(8),
        's' => Some(16),
        'x' => Some(64),
        'a' => Some(256),
        'u' => Some(32),
        _ => None,
    }
}

const RE_TYPE_FLAGS: i32 = 256 | 4 | 32;

/// Find the index of the `)` closing the group opened at `open`
/// (which points at `(`), aware of escapes and character classes.
/// Returns `None` when unbalanced.
fn find_group_end(chars: &[char], open: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut i = open;
    let mut in_class = false;
    let mut class_first = false;
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' {
            i += 2;
            continue;
        }
        if in_class {
            if c == ']' && !class_first {
                in_class = false;
            }
            class_first = false;
            i += 1;
            continue;
        }
        match c {
            '[' => {
                in_class = true;
                class_first = true;
            }
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Parse an inline flag group starting at `open` (the `(` of `(?...`).
/// The char after `(?` is guaranteed to be a flag letter or `-`.
/// Returns (add_bits, del_bits, index of `)` or `:` terminator,
/// scoped?). Errors mirror CPython exactly (message, position, where
/// positions are absolute pattern indices).
fn parse_inline_flags(
    chars: &[char],
    open: usize,
    is_bytes: bool,
) -> Result<(i32, i32, usize, bool), (String, usize)> {
    // Position right after `(?`.
    let mut i = open + 2;
    let mut add_flags = 0i32;
    let mut del_flags = 0i32;
    let get = |i: usize| chars.get(i).copied();
    if get(i) != Some('-') {
        loop {
            let ch = match get(i) {
                Some(c) => c,
                None => return Err(("missing -, : or )".to_string(), i)),
            };
            // Caller guarantees the first char is a flag letter.
            let flag = inline_flag_bit(ch).unwrap();
            if is_bytes {
                if ch == 'u' {
                    return Err((
                        "bad inline flags: cannot use 'u' flag with a bytes pattern"
                            .to_string(),
                        i + 1,
                    ));
                }
            } else if ch == 'L' {
                return Err((
                    "bad inline flags: cannot use 'L' flag with a str pattern".to_string(),
                    i + 1,
                ));
            }
            add_flags |= flag;
            if (flag & RE_TYPE_FLAGS) != 0 && (add_flags & RE_TYPE_FLAGS) != flag {
                return Err((
                    "bad inline flags: flags 'a', 'u' and 'L' are incompatible".to_string(),
                    i + 1,
                ));
            }
            i += 1;
            match get(i) {
                None => return Err(("missing -, : or )".to_string(), i)),
                Some(c) if c == ')' || c == '-' || c == ':' => break,
                Some(c) => match inline_flag_bit(c) {
                    Some(_) => continue,
                    None => {
                        if c.is_alphabetic() {
                            return Err(("unknown flag".to_string(), i));
                        } else {
                            return Err(("missing -, : or )".to_string(), i));
                        }
                    }
                },
            }
        }
    }
    match get(i) {
        Some(')') => Ok((add_flags, 0, i, false)),
        Some('-') => {
            i += 1;
            let ch = match get(i) {
                Some(c) => c,
                None => return Err(("missing flag".to_string(), i)),
            };
            match inline_flag_bit(ch) {
                None => {
                    if ch.is_alphabetic() {
                        return Err(("unknown flag".to_string(), i));
                    } else {
                        return Err(("missing flag".to_string(), i));
                    }
                }
                Some(_) => {}
            }
            loop {
                let ch = get(i).unwrap();
                let flag = inline_flag_bit(ch).unwrap();
                if flag & RE_TYPE_FLAGS != 0 {
                    return Err((
                        "bad inline flags: cannot turn off flags 'a', 'u' and 'L'"
                            .to_string(),
                        i + 1,
                    ));
                }
                del_flags |= flag;
                i += 1;
                match get(i) {
                    None => return Err(("missing :".to_string(), i)),
                    Some(':') => break,
                    Some(c) => match inline_flag_bit(c) {
                        None => {
                            if c.is_alphabetic() {
                                return Err(("unknown flag".to_string(), i));
                            } else {
                                return Err(("missing :".to_string(), i));
                            }
                        }
                        Some(_) => continue,
                    },
                }
            }
            Ok((add_flags, del_flags, i, true))
        }
        Some(':') => Ok((add_flags, del_flags, i, true)),
        // The loop above only exits via break on `)`/`-`/`:`, or
        // returns; `None` is handled inside. Unreachable.
        _ => Err(("missing -, : or )".to_string(), i)),
    }
    .and_then(|(a, d, end, scoped)| {
        if scoped && (a & d) != 0 {
            return Err((
                "bad inline flags: flag turned on and off".to_string(),
                end,
            ));
        }
        Ok((a, d, end, scoped))
    })
}

/// Validate inline flag groups with CPython's exact errors and rewrite
/// `a`/`L` groups (which the engine rejects) into engine-compatible
/// form. Returns (rewritten pattern, pattern-wide ASCII scope from
/// leading global groups, OR of leading global flag bits for
/// combination validation).
///
/// Mirrors CPython `re._parser` flag handling: leading global groups
/// set scope, scoped `(?a:...)`/`(?L:...)` translate their content to
/// ASCII classes, everything else passes through for the engine.
pub(crate) fn preprocess_inline_flags(
    pattern: &str,
    is_bytes: bool,
    flags: i32,
) -> Result<(String, bool, i32), (String, usize)> {
    let chars: Vec<char> = pattern.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    let mut in_class = false;
    let mut class_first = false;
    // Top-level content seen gates global-flag validity. Whitespace
    // and `#`-to-newline comments only count outside VERBOSE mode
    // (mirrors CPython: `' (?i) '` with `re.X` is a valid global);
    // `(?#...)` comments never count.
    let mut verbose_known = flags & 64 != 0;
    let mut seen_content = false;
    let mut depth = 0i32;
    let mut ascii_scope = false;
    let mut global_bits = 0i32;
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' {
            // `\uXXXX`/`\UXXXXXXXX` (unicode escapes) are invalid in BYTES
            // patterns (CPython: `br"\u1234"` → PatternError).
            if is_bytes && i + 1 < chars.len() && (chars[i + 1] == 'u' || chars[i + 1] == 'U') {
                let mut end = i + 2;
                while end < chars.len() && chars[end].is_ascii_hexdigit() {
                    end += 1;
                }
                let esc: String = chars[i..end.min(chars.len())].iter().collect();
                return Err((format!("bad escape {}", esc), i));
            }
            out.push(c);
            if i + 1 < chars.len() {
                out.push(chars[i + 1]);
            }
            if depth == 0 {
                seen_content = true;
            }
            i += 2;
            continue;
        }
        if in_class {
            out.push(c);
            if c == ']' && !class_first {
                in_class = false;
            }
            class_first = false;
            i += 1;
            continue;
        }
        if c == '[' {
            in_class = true;
            class_first = true;
            out.push(c);
            // A class is always content (even in VERBOSE mode, where
            // only bare whitespace/comments are invisible).
            if depth == 0 {
                seen_content = true;
            }
            i += 1;
            continue;
        }
        // A `#` comment runs to the newline in VERBOSE mode (and is
        // invisible to the global gate); otherwise `#` is literal.
        if c == '#' && verbose_known && depth == 0 {
            while i < chars.len() && chars[i] != '\n' {
                out.push(chars[i]);
                i += 1;
            }
            continue;
        }
        if c == '(' {
            if chars.get(i + 1) != Some(&'?') {
                depth += 1;
                if depth == 1 {
                    seen_content = true;
                }
                out.push(c);
                i += 1;
                continue;
            }
            // `(?` group: dispatch on the next char.
            let c2 = chars.get(i + 2).copied();
            match c2 {
                None => {
                    // Lone `(?` at end: leave for the engine.
                    out.push(c);
                    out.push('?');
                    i += 2;
                    continue;
                }
                Some(':') | Some('=') | Some('!') => {
                    // Lookarounds / non-capturing: a group opens;
                    // keep scanning inside.
                    depth += 1;
                    if depth == 1 {
                        seen_content = true;
                    }
                    out.push(c);
                    out.push('?');
                    i += 2;
                    continue;
                }
                Some('#') => {
                    // Comment: skip to `)`, verbatim. Comments never
                    // count as content for the global gate.
                    let mut j = i + 3;
                    while j < chars.len() && chars[j] != ')' {
                        j += 1;
                    }
                    if j < chars.len() {
                        j += 1;
                    }
                    out.push_str(&chars[i..j.min(chars.len())].iter().collect::<String>());
                    i = j.min(chars.len());
                    continue;
                }
                Some('<') => {
                    // `(?<=` / `(?<!` open a group (scan inside);
                    // `(?P<name>` / `(?P=name)` skip to `)` (names hold
                    // no parens). Either way it is content.
                    if chars.get(i + 3) == Some(&'=') || chars.get(i + 3) == Some(&'!') {
                        depth += 1;
                        if depth == 1 {
                            seen_content = true;
                        }
                        out.push(c);
                        out.push('?');
                        i += 2;
                        continue;
                    }
                    let mut j = i + 3;
                    while j < chars.len() && chars[j] != ')' {
                        j += 1;
                    }
                    if j < chars.len() {
                        j += 1;
                    }
                    out.push_str(&chars[i..j.min(chars.len())].iter().collect::<String>());
                    i = j.min(chars.len());
                    if depth == 0 {
                        seen_content = true;
                    }
                    continue;
                }
                Some(f) if inline_flag_bit(f).is_some() || f == '-' => {
                    // Inline flag group: parse exactly like CPython.
                    let (add, del, end, scoped) = parse_inline_flags(&chars, i, is_bytes)?;
                    if !scoped {
                        // Global form: valid only at the very start
                        // (no depth, no content yet).
                        if depth != 0 || seen_content {
                            return Err((
                                "global flags not at the start of the expression".to_string(),
                                i,
                            ));
                        }
                        if add & (256 | 4) != 0 {
                            ascii_scope = true;
                        }
                        global_bits |= add;
                        if add & 64 != 0 {
                            verbose_known = true;
                        }
                        // Valid global groups pass through untouched —
                        // EXCEPT `a`/`L` ones, which the engine rejects:
                        // strip them (ASCII-ness already recorded).
                        if add & (256 | 4) != 0 {
                            i = end + 1;
                            continue;
                        }
                        out.push_str(&chars[i..=end].iter().collect::<String>());
                        i = end + 1;
                        continue;
                    }
                    // Scoped form `(?flags:...)`: locate the close.
                    let close = find_group_end(&chars, i).ok_or_else(|| {
                        // Unterminated: `tell - content_start`.
                        let content = end + 1;
                        (
                            "missing ), unterminated subpattern".to_string(),
                            chars.len().saturating_sub(content),
                        )
                    })?;
                    if depth == 0 {
                        seen_content = true;
                    }
                    if add & (256 | 4) != 0 {
                        // Translate the inner content to ASCII classes.
                        // Remaining flags (if any) survive; dels travel.
                        let inner: String = chars[end + 1..close].iter().collect();
                        let mut head = String::from("(?");
                        for (letter, bit) in
                            [('i', 2), ('m', 8), ('s', 16), ('x', 64), ('u', 32)]
                        {
                            if add & bit != 0 {
                                head.push(letter);
                            }
                        }
                        if del != 0 {
                            head.push('-');
                            for (letter, bit) in [
                                ('i', 2),
                                ('L', 4),
                                ('m', 8),
                                ('s', 16),
                                ('x', 64),
                                ('a', 256),
                                ('u', 32),
                            ] {
                                if del & bit != 0 {
                                    head.push(letter);
                                }
                            }
                        }
                        // `(?a:`/`(?L:` alone become plain `(?:`.
                        head.push(':');
                        let mut rest = head;
                        rest.push_str(&ascii_translate_scoped(&inner));
                        rest.push(')');
                        out.push_str(&rest);
                    } else {
                        out.push_str(&chars[i..=close].iter().collect::<String>());
                    }
                    // Skip past the whole group. Nested flag groups
                    // inside a *translated* region are already
                    // ASCII-covered; inside a *kept* region they would
                    // still need validation — an accepted edge gap
                    // (untested; the engine reports those itself).
                    i = close + 1;
                    continue;
                }
                _ => {
                    // Anything else (`(?)`, `(?+`, ...) — leave for the
                    // engine's own error.
                    out.push(c);
                    out.push('?');
                    i += 2;
                    continue;
                }
            }
        }
        if c == ')' {
            depth = depth.saturating_sub(1);
        }
        // Every other top-level char counts as content for the
        // global gate — except whitespace/comments in VERBOSE mode
        // (`' (?i) '` with `re.X` is valid). Only comments and
        // global flag groups are otherwise exempt (handled above).
        // (Also records a global `(?x...)` for verbose tracking.)
        if depth == 0
            && !(verbose_known && (c == ' ' || c == '\t' || c == '\n' || c == '\r'))
        {
            seen_content = true;
        }
        out.push(c);
        i += 1;
    }
    Ok((out, ascii_scope, global_bits))
}
/// Same as `compile_python_regex`, but applies `re.compile(pattern, flags)`'s
/// `flags` argument — previously accepted and stored on `CompiledRegex` (for
/// `.flags` attribute introspection) but never actually influenced
/// compilation at all, so e.g. `re.IGNORECASE`/`re.VERBOSE`/`re.MULTILINE`/
/// `re.DOTALL` were all silently no-ops. Real trigger: `html.parser`'s own
/// `locatetagend = re.compile(r"""...""", re.VERBOSE)` — a pattern that's
/// entirely unparseable as-is without VERBOSE's whitespace/comment
/// stripping (every space and `# comment` in the triple-quoted pattern is
/// otherwise literal regex syntax). Translated to the regex engine's own
/// inline flag group (`(?ismx)...`) prepended to the pattern — `regex`/
/// `fancy_regex`'s own flag semantics for `i`/`s`/`m`/`x` match Python's
/// IGNORECASE/DOTALL/MULTILINE/VERBOSE closely enough for real-world use.
/// CPython-specific quantifier validation. Returns `Err(offending_pos)` for
/// the first quantifier that has no valid target ("nothing to repeat") or is
/// immediately preceded by another complete quantifier ("multiple repeat").
/// fancy-regex catches plain `*+?` with no target but accepts a leading
/// counted repeat (`{1,2}`) and reads `Q+` as a possessive quantifier — both
/// of which CPython rejects.
fn validate_quantifiers(pattern: &str) -> Result<(), usize> {
    let chars: Vec<char> = pattern.chars().collect();
    let n = chars.len();
    let mut i = 0usize;
    let mut in_class = false;
    let mut class_first = false;
    // `has_atom`: a preceding construct a quantifier may apply to.
    // `after_quant`: a quantifier base has been applied (so another `*`/`{`
    //   here is "multiple repeat").
    // `await_suffix`: we JUST applied a quantifier base, so an immediately
    //   following `?` (lazy) or `+` (possessive, Python 3.11+) is a valid
    //   suffix, not a new quantifier.
    let mut has_atom = false;
    let mut after_quant = false;
    let mut await_suffix = false;
    while i < n {
        let c = chars[i];
        // A pending lazy/possessive suffix is only recognized immediately.
        if await_suffix && (c == '?' || c == '+') {
            await_suffix = false;
            i += 1;
            continue;
        }
        if c != '?' && c != '+' {
            await_suffix = false;
        }
        if c == '\\' {
            has_atom = true;
            after_quant = false;
            i += 2;
            continue;
        }
        if in_class {
            if c == ']' && !class_first {
                in_class = false;
            }
            class_first = false;
            i += 1;
            continue;
        }
        if c == '[' {
            in_class = true;
            class_first = true;
            has_atom = true;
            after_quant = false;
            i += 1;
            continue;
        }
        match c {
            '(' => {
                let mut j = i + 1;
                let mut flags_only = false;
                if j < n && chars[j] == '?' {
                    j += 1;
                    if j < n {
                        match chars[j] {
                            // Non-capturing / lookaround / atomic: body starts
                            // right after the opener.
                            ':' | '=' | '!' | '>' => {
                                j += 1;
                            }
                            '<' => {
                                // `(?<=`/`(?<!` (lookbehind) or `(?<name>`
                                // (named) — body/`>` follows.
                                j += 1;
                                if j < n && (chars[j] == '=' || chars[j] == '!') {
                                    j += 1;
                                } else {
                                    while j < n && chars[j] != '>' {
                                        j += 1;
                                    }
                                    j += 1;
                                }
                            }
                            'P' => {
                                // Python `(?P<name>`, `(?P=name)`, `(?P'name')`.
                                j += 1;
                                if j < n && chars[j] == '<' {
                                    while j < n && chars[j] != '>' {
                                        j += 1;
                                    }
                                    j += 1; // past '>'
                                } else if j < n && (chars[j] == '=' || chars[j] == '\'' || chars[j] == '"')
                                {
                                    // Assertion / conditional — skip to close.
                                    let q = if chars[j] == '=' { ')' } else { chars[j] };
                                    j += 1;
                                    while j < n && chars[j] != q {
                                        j += 1;
                                    }
                                    j += 1;
                                }
                            }
                            '\'' | '"' => {
                                let q = chars[j];
                                j += 1;
                                while j < n && chars[j] != q {
                                    j += 1;
                                }
                                j += 1;
                            }
                            '#' => {
                                while j < n && chars[j] != ')' {
                                    j += 1;
                                }
                                j += 1;
                            }
                            _ => {
                                // Flags `(?flags)` / `(?flags:` — read letters
                                // (and optional `-`) until `:` or `)`.
                                while j < n && chars[j] != ':' && chars[j] != ')' {
                                    j += 1;
                                }
                                if j < n && chars[j] == ':' {
                                    j += 1; // scoped flags: body follows
                                } else if j < n && chars[j] == ')' {
                                    // Global flags `(?i)` — transparent, no atom.
                                    flags_only = true;
                                    j += 1;
                                }
                            }
                        }
                    }
                }
                if flags_only {
                    // Global flags don't change the atom state.
                    i = j;
                    continue;
                }
                has_atom = false;
                after_quant = false;
                i = j;
                continue;
            }
            ')' => {
                has_atom = true;
                after_quant = false;
                i += 1;
                continue;
            }
            '|' => {
                has_atom = false;
                after_quant = false;
                i += 1;
                continue;
            }
            '*' | '+' | '?' => {
                // If we get here with `await_suffix` still set, c was not a
                // suffix — it falls through as a fresh quantifier below
                // (await_suffix was cleared above for non-suffix chars).
                if after_quant {
                    return Err(i); // `*`/`{` (or extra `?`/`+`) after a quantifier
                }
                if !has_atom {
                    return Err(i);
                }
                has_atom = false;
                after_quant = true;
                await_suffix = true;
                i += 1;
                continue;
            }
            '{' => {
                if let Some(end) = counted_repeat_end(&chars, i) {
                    if after_quant {
                        return Err(i);
                    }
                    if !has_atom {
                        return Err(i);
                    }
                    has_atom = false;
                    after_quant = true;
                    await_suffix = true;
                    i = end;
                    continue;
                }
                has_atom = true;
                after_quant = false;
                i += 1;
                continue;
            }
            _ => {
                has_atom = true;
                after_quant = false;
                i += 1;
                continue;
            }
        }
    }
    Ok(())
}

/// If `chars[i..]` starts a valid counted repeat `{n}`/`{n,}`/`{n,m}`, return
/// the index just past the `}`; else `None`. The optional lazy `?` is NOT
/// consumed here — the main loop treats it as a quantifier suffix so that a
/// following `+` is correctly seen as "multiple repeat", not a possessive.
fn counted_repeat_end(chars: &[char], i: usize) -> Option<usize> {
    let n = chars.len();
    if i >= n || chars[i] != '{' {
        return None;
    }
    let mut j = i + 1;
    let mut saw_digit = false;
    while j < n && chars[j].is_ascii_digit() {
        saw_digit = true;
        j += 1;
    }
    if !saw_digit {
        return None;
    }
    if j < n && chars[j] == ',' {
        j += 1;
        while j < n && chars[j].is_ascii_digit() {
            j += 1;
        }
    }
    if j >= n || chars[j] != '}' {
        return None;
    }
    j += 1;
    Some(j)
}

/// CPython group-structure validation: named-group definition/backref
/// checks, duplicate names, and backreferences to still-open groups.
/// Returns `Err((msg, pos))` on the first violation.
fn validate_group_syntax(pattern: &str, is_bytes: bool) -> Result<(), (String, Option<i64>)> {
    fn bad_name_char(name: &str) -> Option<char> {
        // CPython group names: identifier chars, but NOT starting with a digit.
        name.chars().enumerate().find(|(i, c)| {
            if *i == 0 {
                !(c.is_alphabetic() || *c == '_')
            } else {
                !(c.is_alphanumeric() || *c == '_')
            }
        }).map(|(_, c)| c)
    }
    // Bytes patterns: group names must be ASCII; a non-ASCII byte →
    // "bad character in group name '\xNN...'" (test_re: b'(?P<\xc2\xb5>x)').
    // Returns Err(msg, pos) if invalid for the given flavor.
    fn check_name(name: &str, is_bytes: bool, pos: usize) -> Result<(), (String, Option<i64>)> {
        if is_bytes && name.chars().any(|c| !c.is_ascii()) {
            // Escape non-ASCII chars as \xNN (latin-1 byte values).
            let escaped: String = name
                .chars()
                .map(|c| {
                    if c.is_ascii() {
                        c.to_string()
                    } else {
                        format!("\\x{:02x}", c as u32 & 0xff)
                    }
                })
                .collect();
            return Err((
                format!("bad character in group name '{}'", escaped),
                Some(pos as i64),
            ));
        }
        if let Some(bad) = bad_name_char(name) {
            let _ = bad;
            return Err((
                format!("bad character in group name '{}'", name),
                Some(pos as i64),
            ));
        }
        Ok(())
    }
    // Total capturing groups in the WHOLE pattern — conditionals may reference
    // groups defined later (forward refs), so validate against this total.
    let total_groups = count_capturing_groups(pattern);
    let chars: Vec<char> = pattern.chars().collect();
    let n = chars.len();
    let mut named: Vec<(String, usize)> = Vec::new();
    let mut next_group: usize = 0;
    // Stack: Some(group_num) for capturing `(`, None for non-capturing.
    let mut paren_stack: Vec<Option<usize>> = Vec::new();
    let mut open_captures: Vec<usize> = Vec::new();
    let mut i = 0;
    let mut in_class = false;
    let mut class_first = false;
    let is_open = |g: usize, open: &Vec<usize>| open.contains(&g);
    while i < n {
        let c = chars[i];
        if c == '\\' && !in_class && i + 1 < n && chars[i + 1].is_ascii_digit() {
            // Backreference `\N` — reject if group N is still open.
            let mut j = i + 1;
            while j < n && chars[j].is_ascii_digit() {
                j += 1;
            }
            let digits: String = chars[i + 1..j].iter().collect();
            let gnum: usize = digits.chars().take(2).collect::<String>().parse().unwrap_or(0);
            if gnum >= 1 && is_open(gnum, &open_captures) {
                return Err(("cannot refer to an open group".to_string(), Some(i as i64)));
            }
            i = j;
            continue;
        }
        if c == '\\' {
            i += 2;
            continue;
        }
        if in_class {
            if c == ']' && !class_first {
                in_class = false;
            }
            class_first = false;
            i += 1;
            continue;
        }
        if c == '[' {
            in_class = true;
            class_first = true;
            i += 1;
            continue;
        }
        if c == '(' {
            if i + 1 < n && chars[i + 1] == '?' {
                let c2 = chars.get(i + 2).copied();
                match c2 {
                    Some('P') => {
                        let c3 = chars.get(i + 3).copied();
                        if c3 == Some('<') {
                            let name_start = i + 4;
                            let mut j = name_start;
                            while j < n && chars[j] != '>' {
                                j += 1;
                            }
                            if j >= n {
                                return Err((
                                    "missing >, unterminated name".to_string(),
                                    Some(name_start as i64),
                                ));
                            }
                            let name: String = chars[name_start..j].iter().collect();
                            if name.is_empty() {
                                return Err((
                                    "missing group name".to_string(),
                                    Some(name_start as i64),
                                ));
                            }
                            check_name(&name, is_bytes, name_start)?;
                            if let Some((_, prev)) = named.iter().find(|(nm, _)| *nm == name) {
                                return Err((
                                    format!(
                                        "redefinition of group name '{}' as group {}; was group {}",
                                        name,
                                        next_group + 1,
                                        prev
                                    ),
                                    None,
                                ));
                            }
                            next_group += 1;
                            named.push((name, next_group));
                            paren_stack.push(Some(next_group));
                            open_captures.push(next_group);
                            i = j + 1;
                            continue;
                        } else if c3 == Some('=') {
                            // Named backref `(?P=name)`.
                            let name_start = i + 4;
                            let mut j = name_start;
                            while j < n && chars[j] != ')' {
                                j += 1;
                            }
                            if j >= n {
                                return Err((
                                    "missing ), unterminated name".to_string(),
                                    Some(name_start as i64),
                                ));
                            }
                            let name: String = chars[name_start..j].iter().collect();
                            if name.is_empty() {
                                return Err((
                                    "missing group name".to_string(),
                                    Some(name_start as i64),
                                ));
                            }
                            check_name(&name, is_bytes, name_start)?;
                            match named.iter().find(|(nm, _)| *nm == name) {
                                None => {
                                    return Err((
                                        format!("unknown group name '{}'", name),
                                        None,
                                    ))
                                }
                                Some((_, gnum)) => {
                                    if is_open(*gnum, &open_captures) {
                                        return Err((
                                            "cannot refer to an open group".to_string(),
                                            Some((i + 4) as i64),
                                        ));
                                    }
                                }
                            }
                            paren_stack.push(None);
                            i = j + 1;
                            continue;
                        } else {
                            return Err(("unknown extension ?Px".to_string(), None));
                        }
                    }
                    Some('<') => {
                        // `(?<=`/`(?<!` lookbehind, or `(?<name>` (Py3.12 extended).
                        let c3 = chars.get(i + 3).copied();
                        if c3 == Some('=') || c3 == Some('!') {
                            paren_stack.push(None);
                            i += 4; // past (?<= / (?<!
                            continue;
                        }
                        // `(?<name>` named group (extended syntax).
                        let name_start = i + 3;
                        let mut j = name_start;
                        while j < n && chars[j] != '>' {
                            j += 1;
                        }
                        if j >= n {
                            return Err((
                                "missing >, unterminated name".to_string(),
                                Some(name_start as i64),
                            ));
                        }
                        let name: String = chars[name_start..j].iter().collect();
                        if name.is_empty() {
                            return Err((
                                "missing group name".to_string(),
                                Some(name_start as i64),
                            ));
                        }
                        check_name(&name, is_bytes, name_start)?;
                        if let Some((_, prev)) = named.iter().find(|(nm, _)| *nm == name) {
                            return Err((
                                format!(
                                    "redefinition of group name '{}' as group {}; was group {}",
                                    name,
                                    next_group + 1,
                                    prev
                                ),
                                None,
                            ));
                        }
                        next_group += 1;
                        named.push((name, next_group));
                        paren_stack.push(Some(next_group));
                        open_captures.push(next_group);
                        i = j + 1;
                        continue;
                    }
                    Some(':') | Some('=') | Some('!') | Some('#') => {
                        paren_stack.push(None);
                        i += 3;
                        continue;
                    }
                    Some('(') => {
                        // Conditional `(?(cond)...)` — validate the condition.
                        let cond_start = i + 3;
                        let mut j = cond_start;
                        while j < n && chars[j] != ')' {
                            j += 1;
                        }
                        if j >= n {
                            // Condition's `)` never found — the group-ref
                            // NAME inside the conditional is unterminated
                            // (`()(?(1` → "missing ), unterminated name").
                            return Err((
                                "missing ), unterminated name".to_string(),
                                Some(cond_start as i64),
                            ));
                        }
                        let cond: String = chars[cond_start..j].iter().collect();
                        if cond.is_empty() {
                            return Err((
                                "missing group name".to_string(),
                                Some(cond_start as i64),
                            ));
                        }
                        if cond.chars().all(|c| c.is_ascii_digit()) {
                            let num: usize = cond.parse().unwrap_or(0);
                            if num == 0 {
                                return Err((
                                    "bad group number".to_string(),
                                    Some(cond_start as i64),
                                ));
                            }
                            if num > total_groups {
                                return Err((
                                    format!("invalid group reference {}", num),
                                    Some(cond_start as i64),
                                ));
                            }
                        } else {
                            check_name(&cond, is_bytes, cond_start)?;
                            if named.iter().find(|(nm, _)| *nm == cond).is_none() {
                                return Err((
                                    format!("unknown group name '{}'", cond),
                                    None,
                                ));
                            }
                        }
                        // More than two branches in the conditional body is an error
                        // (`(?(1)a|b|c)` → "conditional backref with more
                        // than two branches", test_re).
                        {
                            let mut k = j + 1;
                            let mut depth = 1i32;
                            let mut pipes = 0;
                            let mut closed = false;
                            while k < n {
                                let ch = chars[k];
                                if ch == '\\' {
                                    k += 2;
                                    continue;
                                }
                                if ch == '(' {
                                    depth += 1;
                                } else if ch == ')' {
                                    depth -= 1;
                                    if depth == 0 {
                                        closed = true;
                                        break;
                                    }
                                } else if ch == '|' && depth == 1 {
                                    pipes += 1;
                                    if pipes >= 2 {
                                        return Err((
                                        "conditional backref with more than two branches"
                                                .to_string(),
                                            Some(k as i64),
                                        ));
                                    }
                                }
                                k += 1;
                            }
                            if !closed {
                                // Conditional never closed — body unterminated
                                // (`()(?(1)a` → "missing ), unterminated
                                // subpattern" at the conditional's `(`).
                                return Err((
                                    "missing ), unterminated subpattern".to_string(),
                                    Some(i as i64),
                                ));
                            }
                        }
                        paren_stack.push(None);
                        i = j + 1; // past the condition's `)`; body follows.
                        continue;
                    }
                    _ => {
                        // Flags `(?i)` / `(?flags:` — treat as non-capturing.
                        paren_stack.push(None);
                        i += 2;
                        continue;
                    }
                }
            }
            // Plain capturing group.
            next_group += 1;
            paren_stack.push(Some(next_group));
            open_captures.push(next_group);
            i += 1;
            continue;
        }
        if c == ')' {
            if let Some(gnum) = paren_stack.pop() {
                if let Some(g) = gnum {
                    open_captures.retain(|&x| x != g);
                }
            }
            i += 1;
            continue;
        }
        i += 1;
    }
    Ok(())
}

/// Scan for octal escapes whose value exceeds 0o377 and return the byte
/// position + digit string of the first offender (CPython raises
/// "octal escape value \NNN outside of range0-0o377" during parse).
fn find_octal_range_error(pattern: &str) -> Option<(usize, String)> {
    let chars: Vec<char> = pattern.chars().collect();
    let mut byte_pos = 0usize;
    let mut i = 0usize;
    let mut in_class = false;
    let mut class_first = false;
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' && i + 1 < chars.len() {
            let nxt = chars[i + 1];
            if nxt.is_ascii_digit() {
                // Determine octal digits CPython would consume.
                let oct_digits: String;
                if nxt == '0' {
                    let mut d = String::from("0");
                    let mut k = i + 2;
                    let mut cnt = 1;
                    while k < chars.len() && cnt < 3 && chars[k].is_digit(8) {
                        d.push(chars[k]);
                        k += 1;
                        cnt += 1;
                    }
                    oct_digits = d;
                } else if nxt.is_digit(8)
                    && i + 2 < chars.len()
                    && chars[i + 2].is_digit(8)
                    && i + 3 < chars.len()
                    && chars[i + 3].is_digit(8)
                {
                    oct_digits = chars[i + 1..i + 4].iter().collect();
                } else {
                    oct_digits = String::new();
                }
                if !oct_digits.is_empty() {
                    if let Ok(val) = u32::from_str_radix(&oct_digits, 8) {
                        if val > 0o377 {
                            return Some((byte_pos, oct_digits));
                        }
                    }
                }
            }
            byte_pos += 2;
            i += 2;
            continue;
        }
        if c == '[' {
            in_class = true;
            class_first = true;
        } else if c == ']' && !class_first {
            in_class = false;
        }
        class_first = false;
        byte_pos += c.len_utf8();
        i += 1;
    }
    None
}

pub fn compile_python_regex_flags(
    pattern: &str,
    flags: i32,
    is_bytes: bool,
) -> Result<fancy_regex::Regex, fancy_regex::Error> {
    // CPython-specific quantifier validation (see `validate_quantifiers`).
    if let Err(char_idx) = validate_quantifiers(pattern) {
        // validate works in CHAR indices; fancy's ParseError carries a BYTE
        // offset (which `fancy_error_to_pattern` converts back to a char
        // index for `.pos`). Round-trip so multi-byte patterns stay correct.
        let byte_idx = pattern
            .char_indices()
            .nth(char_idx)
            .map(|(b, _)| b)
            .unwrap_or(pattern.len());
        return Err(fancy_regex::Error::ParseError(
            byte_idx,
            fancy_regex::ParseError::TargetNotRepeatable,
        ));
    }
    // Group-structure validation (duplicate names, open-group backrefs, etc).
    if let Err((msg, pos)) = validate_group_syntax(pattern, is_bytes) {
        return Err(fancy_regex::Error::ParseError(
            pos.unwrap_or(0) as usize,
            fancy_regex::ParseError::GeneralParseError(msg),
        ));
    }
    // Octal-escape range check (CPython raises at parse time).
    if let Some((byte_pos, digits)) = find_octal_range_error(pattern) {
        return Err(fancy_regex::Error::ParseError(
            byte_pos,
            fancy_regex::ParseError::GeneralParseError(format!(
                "octal escape value \\{} outside of range 0-0o377",
                digits
            )),
        ));
    }
    let pattern = escape_loose_braces(pattern);
    let pattern = escape_leading_bracket_in_class(&pattern);
    let pattern = translate_pattern_backrefs_and_octal(&pattern);
    // CPython: a non-multiline `$` matches at end-of-string *and* just
    // before a single trailing newline. The engine's `$` only matches
    // at the very end, so rewrite it to an explicit two-way anchor.
    // `\$` and `$` inside character classes stay literal; patterns with
    // an inline `(?m` already get engine (i.e. multiline) `$` semantics.
    let pattern = if flags & 8 == 0 && !pattern.contains("(?m") {
        translate_dollar_anchor(&pattern)
    } else {
        pattern
    };
    // The engine's `(?x)` ignores space/tab/CR/LF but not `\v`/`\f`
    // (CPython VERBOSE strips `[ \t\n\r\f\v]`). Strip those two
    // textually when outside a character class (where they'd be
    // literal) and outside a `#` comment (which already runs to \n).
    let pattern = if flags & 64 != 0 {
        verbose_strip_vf(&pattern)
    } else {
        pattern
    };
    let mut inline = String::new();
    if flags & 2 != 0 {
        inline.push('i');
    } // IGNORECASE
    if flags & 16 != 0 {
        inline.push('s');
    } // DOTALL
    if flags & 8 != 0 {
        inline.push('m');
    } // MULTILINE
    if flags & 64 != 0 {
        inline.push('x');
    } // VERBOSE
    let pattern = if inline.is_empty() {
        pattern
    } else {
        format!("(?{}){}", inline, pattern)
    };
    // ASCII (`(?a)` / `re.ASCII` = 256) without UNICODE (32): `\w`,
    // `\b`, `\d`, `\s` match ASCII only (mirrors CPython; e.g.
    // `re.search(r"\b(ьюя)\b", "ьюя", re.ASCII)` is None). The
    // engine has no `(?-u)` switch ("Disabling Unicode not
    // supported"), so translate textually. Nested `(?u:` scopes
    // keep Unicode classes (mirrors CPython scoping).
    let pattern = if flags & 256 != 0 && flags & 32 == 0 {
        ascii_translate_scoped(&pattern)
    } else {
        pattern
    };
    let pattern = translate_bare_name_conditionals(&pattern);
    fancy_regex::Regex::new(&pattern)
}

/// fancy-regex only treats a conditional's target as a group-existence
/// check when it is delimited (`(?(<name>)` / `(?('name')`) or numeric;
/// a BARE name `(?(name)...)` falls through to a plain sub-expression
/// parse and is read as a literal, so the condition always evaluated
/// false (CPython treats a bare name as "did group `name` match").
/// Rewrite a bare ASCII group name to the `<...>` form fancy-regex
/// understands. Numeric/relative/quoted/`DEFINE`/`(*verb)` targets are
/// left untouched (they already dispatch correctly).
fn translate_bare_name_conditionals(pattern: &str) -> String {
    let chars: Vec<char> = pattern.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    let mut in_class = false;
    let mut class_first = false;
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' {
            out.push(c);
            if i + 1 < chars.len() {
                out.push(chars[i + 1]);
            }
            i += 2;
            continue;
        }
        if in_class {
            out.push(c);
            if c == ']' && !class_first {
                in_class = false;
            }
            class_first = false;
            i += 1;
            continue;
        }
        if c == '[' {
            in_class = true;
            class_first = true;
            out.push(c);
            i += 1;
            continue;
        }
        // `(?(` opens a conditional.
        if c == '(' && i + 2 < chars.len() && chars[i + 1] == '?' && chars[i + 2] == '(' {
            let after = i + 3;
            let mut wrapped = false;
            if after < chars.len() && (chars[after].is_ascii_alphabetic() || chars[after] == '_') {
                let mut j = after;
                while j < chars.len() && (chars[j].is_ascii_alphanumeric() || chars[j] == '_') {
                    j += 1;
                }
                let name: String = chars[after..j].iter().collect();
                if name != "DEFINE" && j < chars.len() && chars[j] == ')' {
                    out.push_str("(?(");
                    out.push('<');
                    out.push_str(&name);
                    out.push_str(">)");
                    i = j + 1;
                    wrapped = true;
                }
            }
            if !wrapped {
                out.push_str("(?(");
                i += 3;
            }
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

/// Remove `\v`/`\f` outside character classes and `#` comments
/// (see `compile_python_regex_flags`).
fn verbose_strip_vf(pattern: &str) -> String {
    let chars: Vec<char> = pattern.chars().collect();
    let mut out = String::new();
    let mut in_class = false;
    let mut in_comment = false;
    let mut escaped = false;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if in_comment {
            if c == '\n' {
                in_comment = false;
            }
        } else if !escaped {
            match c {
                '#' => in_comment = true,
                '[' => in_class = true,
                ']' if in_class => in_class = false,
                '\u{b}' | '\u{c}' if !in_class => {
                    i += 1;
                    continue;
                }
                _ => {}
            }
        }
        if escaped {
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        }
        out.push(c);
        i += 1;
    }
    out
}

/// CPython's non-multiline `$` matches at end-of-string and just before a
/// single trailing newline; rewrite to an explicit two-way anchor. Escaped
/// `\$` and `$` inside character classes stay literal.
fn translate_dollar_anchor(pattern: &str) -> String {
    let chars: Vec<char> = pattern.chars().collect();
    let mut out = String::with_capacity(pattern.len());
    let mut i = 0;
    let mut in_class = false;
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' {
            out.push(c);
            if i + 1 < chars.len() {
                out.push(chars[i + 1]);
            }
            i += 2;
            continue;
        }
        if c == '[' {
            in_class = true;
        } else if c == ']' && in_class {
            in_class = false;
        }
        if c == '$' && !in_class {
            out.push_str("(?:(?=\\n\\z)|\\z)");
        } else {
            out.push(c);
        }
        i += 1;
    }
    out
}

/// Split `text` at top-level `(?u:` / `(?a:` / `(?L:` groups.
/// Returns (segment, kind) with kind 0 = plain text, 1 = `u`-group,
/// 2 = `a`/`L`-group (whole span including prefix and closer).
/// Escape-, class- and nesting-aware.
fn split_top_flag_groups(text: &str) -> Vec<(String, u8)> {
    let chars: Vec<char> = text.chars().collect();
    let mut out: Vec<(String, u8)> = Vec::new();
    let mut cur = String::new();
    let mut i = 0;
    let mut depth = 0i32;
    let mut in_class = false;
    let mut class_first = false;
    let starts_with_at = |j: usize, s: &str| -> bool {
        let v: Vec<char> = s.chars().collect();
        j + v.len() <= chars.len() && chars[j..j + v.len()] == v[..]
    };
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' {
            cur.push(c);
            if i + 1 < chars.len() {
                cur.push(chars[i + 1]);
            }
            i += 2;
            continue;
        }
        if in_class {
            cur.push(c);
            if c == ']' && !class_first {
                in_class = false;
            }
            class_first = false;
            i += 1;
            continue;
        }
        if c == '[' {
            in_class = true;
            class_first = true;
            cur.push(c);
            i += 1;
            continue;
        }
        if c == '(' {
            if starts_with_at(i, "(?")
                && depth == 0
            {
                let mut j = i + 2;
                let mut letters = String::new();
                while j < chars.len()
                    && (chars[j].is_ascii_alphabetic() || chars[j] == '-')
                {
                    letters.push(chars[j]);
                    j += 1;
                }
                if chars.get(j) == Some(&':')
                    && (letters.contains('u')
                        || letters.contains('a')
                        || letters.contains('L'))
                {
                    if !cur.is_empty() {
                        out.push((std::mem::take(&mut cur), 0));
                    }
                    let mut k = i;
                    let mut d = 0i32;
                    let mut ic = false;
                    let mut cf = false;
                    let mut end = chars.len();
                    while k < chars.len() {
                        let cc = chars[k];
                        if cc == '\\' {
                            k += 2;
                            continue;
                        }
                        if ic {
                            if cc == ']' && !cf {
                                ic = false;
                            }
                            cf = false;
                            k += 1;
                            continue;
                        }
                        if cc == '[' {
                            ic = true;
                            cf = true;
                            k += 1;
                            continue;
                        }
                        if cc == '(' {
                            d += 1;
                        } else if cc == ')' {
                            d -= 1;
                            if d == 0 {
                                end = k;
                                break;
                            }
                        }
                        k += 1;
                    }
                    let kind = if letters.contains('u')
                        && !letters.contains('a')
                        && !letters.contains('L')
                    {
                        1u8
                    } else {
                        2u8
                    };
                    let span: String = chars[i..=end.min(chars.len() - 1)].iter().collect();
                    out.push((span, kind));
                    i = end.min(chars.len() - 1) + 1;
                    continue;
                }
            }
            depth += 1;
            cur.push(c);
            i += 1;
            continue;
        }
        if c == ')' {
            depth = depth.saturating_sub(1);
        }
        cur.push(c);
        i += 1;
    }
    if !cur.is_empty() {
        out.push((cur, 0));
    }
    out
}

/// Split a flag-group span into (prefix_through_colon, body): the
/// prefix ends at the first `:` (flag letters hold no colon), the
/// body excludes the final `)`. Falls back to (whole, empty) when
/// malformed (shouldn't happen — spans come from the splitter).
fn split_group_prefix(seg: &str) -> (String, String) {
    let chars: Vec<char> = seg.chars().collect();
    let mut j = 3;
    while j < chars.len() && chars[j] != ':' {
        j += 1;
    }
    if j + 1 >= chars.len() {
        return (seg.to_string(), String::new());
    }
    let prefix: String = chars[..=j].iter().collect();
    let body: String = if j + 1 < chars.len().saturating_sub(1) {
        chars[j + 1..chars.len() - 1].iter().collect()
    } else {
        String::new()
    };
    (prefix, body)
}

/// Translate an `(?a:`/`(?L:` scope body to ASCII classes, leaving
/// nested `(?u:` scopes verbatim (they restore Unicode), recursing
/// into doubly-nested `(?a:` scopes.
fn ascii_translate_scoped(text: &str) -> String {
    let mut out = String::new();
    for (seg, kind) in split_top_flag_groups(text) {
        match kind {
            // `u`-group: keep prefix, recurse into the body for
            // nested `(?a:` scopes, keep the closer.
            1 => {
                let (prefix, body) = split_group_prefix(&seg);
                out.push_str(&prefix);
                out.push_str(&translate_u_body(&body));
                out.push(')');
            }
            // Nested `a`-group: translate like the outer scope.
            2 => {
                let (prefix, body) = split_group_prefix(&seg);
                out.push_str(&prefix);
                out.push_str(&ascii_translate_scoped(&body));
                out.push(')');
            }
            _ => {
                out.push_str(&ascii_word_classes(&seg));
            }
        }
    }
    out
}

/// Translate a `(?u:` scope body: gaps stay verbatim, nested
/// `(?a:`/`(?L:` groups translate (recursing for deeper `(?u:`).
fn translate_u_body(text: &str) -> String {
    let mut out = String::new();
    for (seg, kind) in split_top_flag_groups(text) {
        match kind {
            2 => {
                let (prefix, body) = split_group_prefix(&seg);
                out.push_str(&prefix);
                out.push_str(&ascii_translate_scoped(&body));
                out.push(')');
            }
            _ => out.push_str(&seg),
        }
    }
    out
}

/// Rewrite `\b \B \w \W \d \D \s \S` to ASCII-only equivalents for
/// `re.ASCII` (see above). Respects escapes, `\\` pairs and `[...]`
/// spans (where `\b` is backspace, left untouched).
pub(crate) fn ascii_word_classes(pattern: &str) -> String {
    const WORD: &str = "A-Za-z0-9_";
    let mut out = String::with_capacity(pattern.len());
    let chars: Vec<char> = pattern.chars().collect();
    let mut i = 0;
    let mut in_class = false;
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' && i + 1 < chars.len() {
            let n = chars[i + 1];
            match n {
                'w' => {
                    out.push_str("[A-Za-z0-9_]");
                    i += 2;
                    continue;
                }
                'W' => {
                    out.push_str("[^A-Za-z0-9_]");
                    i += 2;
                    continue;
                }
                'd' => {
                    out.push_str("[0-9]");
                    i += 2;
                    continue;
                }
                'D' => {
                    out.push_str("[^0-9]");
                    i += 2;
                    continue;
                }
                's' => {
                    out.push_str("[ \\t\\n\\r\\f\\v]");
                    i += 2;
                    continue;
                }
                'S' => {
                    out.push_str("[^ \\t\\n\\r\\f\\v]");
                    i += 2;
                    continue;
                }
                'b' if !in_class => {
                    out.push_str(&format!(
                        "(?:(?<=[{w}])(?![{w}])|(?<![{w}])(?=[{w}]))",
                        w = WORD
                    ));
                    i += 2;
                    continue;
                }
                'B' if !in_class => {
                    out.push_str(&format!(
                        "(?:(?<=[{w}])(?=[{w}])|(?<![{w}])(?![{w}]))",
                        w = WORD
                    ));
                    i += 2;
                    continue;
                }
                _ => {
                    out.push(c);
                    out.push(n);
                    i += 2;
                    continue;
                }
            }
        }
        if c == '[' && !in_class {
            // A `[` preceded by `\` was consumed above; a bare `[`
            // opens a class (POSIX nesting quirks are handled by the
            // earlier bracket pass).
            in_class = true;
            out.push(c);
            i += 1;
            // `[]` / `[^]` lead with a literal `]`.
            if i < chars.len() && chars[i] == '^' {
                out.push('^');
                i += 1;
            }
            if i < chars.len() && chars[i] == ']' {
                out.push(']');
                i += 1;
            }
            continue;
        }
        if c == ']' && in_class {
            in_class = false;
        }
        out.push(c);
        i += 1;
    }
    out
}
