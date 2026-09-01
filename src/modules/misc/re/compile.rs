use crate::object::*;
use std::collections::HashMap;

pub fn re_pattern_error(msg: String, pattern: Option<String>, pos: Option<i64>) -> PyError {
    let mut extra = HashMap::new();
    extra.insert("msg".to_string(), py_str(&msg));
    extra.insert(
        "pattern".to_string(),
        match pattern {
            Some(p) => py_str(&p),
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
    let exc = PyObjectRef::new(PyObject::Exception {
        typ: "PatternError".to_string(),
        args: vec![py_str(&msg)],
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
                                i = k;
                                for idx in k..j {
                                    out.push(chars[idx]);
                                }
                                continue;
                            }
                        }
                    }
                    out.push(c);
                    out.push(nxt);
                    i += 2;
                    continue;
                }
                if digits.len() == 3 && digits.chars().all(|ch| ch.is_digit(8)) {
                    if let Ok(val) = u32::from_str_radix(&digits, 8) {
                        if val <= 0o377 {
                            out.push_str(&format!("\\x{{{:x}}}", val));
                            i = j;
                            continue;
                        }
                    }
                }
                let num: usize = digits.parse().unwrap_or(0);
                if num != 0 && num <= num_groups {
                    out.push(c);
                    out.extend(digits.chars());
                    i = j;
                    continue;
                } else if num > num_groups && num_groups > 0 {
                    let mut found = None;
                    for len in (1..digits.len()).rev() {
                        if let Ok(prefix) = digits[..len].parse::<usize>() {
                            if prefix != 0 && prefix <= num_groups {
                                found = Some(len);
                                break;
                            }
                        }
                    }
                    if let Some(len) = found {
                        out.push_str(&format!("\\g<{}>", &digits[..len]));
                        out.extend(digits[len..].chars());
                        i = j;
                        continue;
                    }
                }
                out.push(c);
                out.extend(digits.chars());
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

pub fn translate_python_replacement(repl: &str) -> String {
    let chars: Vec<char> = repl.chars().collect();
    let mut out = String::with_capacity(repl.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '$' {
            out.push_str("$$");
            i += 1;
        } else if c == '\\' && i + 1 < chars.len() {
            let next = chars[i + 1];
            if next.is_ascii_digit() {
                let mut j = i + 1;
                while j < chars.len() && chars[j].is_ascii_digit() {
                    j += 1;
                }
                out.push_str("${");
                out.extend(&chars[i + 1..j]);
                out.push('}');
                i = j;
            } else if next == 'g' && chars.get(i + 2) == Some(&'<') {
                let mut j = i + 3;
                while j < chars.len() && chars[j] != '>' {
                    j += 1;
                }
                out.push_str("${");
                out.extend(&chars[i + 3..j]);
                out.push('}');
                i = if j < chars.len() { j + 1 } else { j };
            } else if next == '\\' {
                out.push('\\');
                i += 2;
            } else {
                out.push(next);
                i += 2;
            }
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

pub fn validate_g_template(repl: &str, re: &fancy_regex::Regex) -> PyResult<()> {
    let chars: Vec<char> = repl.chars().collect();
    let mut i = 0;
    let n_groups = re.capture_names().count().saturating_sub(1);
    while i < chars.len() {
        if chars[i] == '\\' && i + 1 < chars.len() {
            let nxt = chars[i + 1];
            if nxt == 'g' {
                if i + 2 >= chars.len() || chars[i + 2] != '<' {
                    return Err(re_pattern_error("missing <".to_string(), Some(repl.to_string()), Some(2)));
                }
                let mut j = i + 3;
                while j < chars.len() && chars[j] != '>' {
                    j += 1;
                }
                if j >= chars.len() {
                    return Err(re_pattern_error("missing >, unterminated name".to_string(), Some(repl.to_string()), Some(3)));
                }
                let name: String = chars[i + 3..j].iter().collect();
                if name.is_empty() {
                    return Err(re_pattern_error("missing group name".to_string(), Some(repl.to_string()), Some(3)));
                }
                let mut bad = false;
                for (idx, ch) in name.chars().enumerate() {
                    if idx == 0 && ch.is_ascii_digit() {
                        if let Ok(num) = name.parse::<usize>() {
                            if num == 0 || num > n_groups {
                                return Err(re_pattern_error(format!("invalid group reference {}", num), Some(repl.to_string()), Some(3)));
                            }
                        } else {
                            bad = true;
                        }
                        break;
                    }
                    if !(ch.is_alphanumeric() || ch == '_') {
                        bad = true;
                        break;
                    }
                }
                if name.contains(' ') || name.contains('<') || name.contains('>') {
                    bad = true;
                }
                if bad {
                    return Err(re_pattern_error(format!("bad character in group name '{}'", name), Some(repl.to_string()), Some(3)));
                }
                i = j + 1;
                continue;
            } else if nxt.is_ascii_digit() {
                let mut j = i + 1;
                while j < chars.len() && chars[j].is_ascii_digit() {
                    j += 1;
                }
                let num_str: String = chars[i + 1..j].iter().collect();
                if num_str.starts_with('0') {
                    i = j;
                    continue;
                }
                if let Ok(num) = num_str.parse::<usize>() {
                    if num == 0 || num > n_groups {
                        return Err(re_pattern_error(format!("invalid group reference {}", num), Some(repl.to_string()), Some(1)));
                    }
                }
                i = j;
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

pub fn compile_python_regex(pattern: &str) -> Result<fancy_regex::Regex, fancy_regex::Error> {
    compile_python_regex_flags(pattern, 0)
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
pub fn compile_python_regex_flags(
    pattern: &str,
    flags: i32,
) -> Result<fancy_regex::Regex, fancy_regex::Error> {
    let pattern = escape_loose_braces(pattern);
    let pattern = escape_leading_bracket_in_class(&pattern);
    let pattern = translate_pattern_backrefs_and_octal(&pattern);
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
    fancy_regex::Regex::new(&pattern)
}
